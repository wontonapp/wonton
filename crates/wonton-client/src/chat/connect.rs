//! 登录、断开，以及登录因为暂时的原因没成功时按退避自动重登。
//!
//! 家里的连接一直连着、只是那一次登录请求失败（满载时要令牌超时、网关回 502、Matrix 还在启动）时，`follow_home` 看不到
//! 任何变化，不会再登录；不自己重试的话聊天就一直断着，直到 App 回到前台。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use wonton_protocol::{ChatAccount, ChatFailure, ChatSync, HomeStatus, ProxySettings};

use super::link::Active;
use super::{ChatLink, failure, lock};

/// 登录到哪里：经家里的本地代理登录配对的服务器，或者直连外部 Matrix 账号的 homeserver（ADR-0051）。
/// `HomeOffline` 是本机还有那台服务器的会话、本地代理却还没起来（隧道没通）：先不带代理恢复，代理来了再由带代理的
/// 那次接上。
#[derive(Clone, PartialEq, Eq)]
pub(super) enum Target {
    Home(ProxySettings),
    HomeOffline,
    External,
}

/// 隧道正在建时，离线恢复最多先等这么久本地代理：等到了就一次恢复带代理，省掉在同一个存储上恢复两次（两次是
/// 两套设备身份，对方要重新认一遍密钥）。真连不上就用本机存下的会话先恢复。
const OFFLINE_GRACE: Duration = Duration::from_millis(500);

/// 第一次重试前等这么久，之后每次翻倍，最多等 [`MAX_BACKOFF`]。
const FIRST_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// 自动重登的进度。每次开始登录都换一代：还在等的重试醒来发现代数变了就作罢，同一时刻最多一路重试在等。
#[derive(Default)]
pub(super) struct Relogin(Mutex<ReloginState>);

#[derive(Default)]
struct ReloginState {
    generation: u64,
    /// 连续几次因为暂时的原因没登录上，登录成功或遇到要用户处理的失败时清零。
    failures: u32,
}

/// 连不上、Matrix 还没就绪、对方公网关会自己好；没配对、被拒、输入非法、服务器出错要用户处理，重试也没用。
fn temporary(reason: ChatFailure) -> bool {
    matches!(
        reason,
        ChatFailure::TransportUnavailable
            | ChatFailure::PeerPublicAccessOff
            | ChatFailure::NotReady
    )
}

fn backoff(failures: u32) -> Duration {
    FIRST_BACKOFF
        .saturating_mul(1 << failures.saturating_sub(1).min(16))
        .min(MAX_BACKOFF)
}

impl ChatLink {
    /// 登录或恢复会话，然后开始同步。
    pub(super) async fn connect(self: Arc<Self>, target: Target) {
        let generation = {
            let mut relogin = lock(&self.relogin.0);
            relogin.generation += 1;
            relogin.generation
        };
        let mut active = self.active.lock().await;
        // 等锁时又开始了一次登录（比如等着重连时回到前台）或断开了：由后来的那次决定，不再登录一遍。
        if lock(&self.relogin.0).generation != generation {
            return;
        }
        // 正在（或已经）用这个代理登录：隧道刚通、代理刚到，本机那次恢复也是用它，不用再换一次会话。
        // 失败过（包括设备被吊销）不算，那要重新登录。
        if let Target::Home(proxy) = &target
            && lock(&self.signing_proxy).as_ref() == Some(proxy)
            && !matches!(self.view.borrow().account, ChatAccount::Failed { .. })
        {
            return;
        }
        // 等锁时可能又换了连接：这个代理已经没了，不能凭它动下一台服务器的登录状态。外部账号的槽位一个槽位一个账号，
        // 账号数据按槽位记。
        let (server_id, proxy) = match &target {
            Target::Home(proxy) => {
                let home = self.home.view();
                let Some(server_id) = home
                    .server_id
                    .filter(|_| home.proxy.as_ref() == Some(proxy))
                else {
                    return;
                };
                (server_id, Some(proxy.clone()))
            }
            // 没有本地代理：隧道多半正在建，给它一小会儿，代理到了就直接用代理登录。真没有代理就用本机存着的那台
            // 会话先恢复出来（请求连不上按传输错误失败）。没有能恢复的会话就交给带代理的那次。
            Target::HomeOffline => {
                let home = self.home.view();
                let proxy = match (home.proxy, home.status) {
                    (None, HomeStatus::Connecting) => self.home.wait_for_proxy(OFFLINE_GRACE).await,
                    (proxy, _) => proxy,
                };
                let Some(server_id) = home.server_id.filter(|id| self.has_local_session(id)) else {
                    return;
                };
                (server_id, proxy)
            }
            Target::External => (String::new(), None),
        };
        // 会话马上要让位（清理本身可能等 2 秒），视图先改成“正在登录”：这期间不能还挂着上一次的 SignedIn，
        // 否则要会话的动作都回 `NotReady`、界面却显示已登录。
        self.view
            .send_modify(|view| view.account = ChatAccount::SigningIn);
        if let Some(old) = active.take() {
            self.forget_current().await;
            drop(old);
        }
        // 清理这会儿隧道可能又换过代理（抖动时每 1–3 秒一次）：用眼下最新的那个登录，别拿一个已经拆掉的旧地址
        // 去 restore。走的时候还有代理、这时没了就交给 follow_home 的 `None` 分支（它会断开会话、离线恢复）。
        let proxy = match target {
            Target::External => proxy,
            Target::Home(_) if self.home.view().proxy.is_none() => return,
            _ => self.home.view().proxy.clone().or(proxy),
        };
        *lock(&self.signing_proxy) = proxy.clone();
        tracing::info!(server = %server_id, proxy = proxy.is_some(), "Matrix sign-in starting");
        // 账号数据跟着服务器走：断线时先留着，换服务器才清。
        if self.layouts.adopt(&server_id) {
            self.view
                .send_modify(|view| view.chat_layout_default = Default::default());
        }
        let signed_in = match &target {
            Target::External => self.sign_in_external().await,
            _ => self.sign_in(proxy.as_ref(), server_id).await,
        };
        let session = match signed_in {
            Ok(session) => Arc::new(session),
            Err((reason, message)) => {
                tracing::info!("Matrix sign-in failed: {message}");
                *lock(&self.signing_proxy) = None;
                drop(active);
                self.sign_in_failed(generation, target, reason, message);
                return;
            }
        };
        lock(&self.relogin.0).failures = 0;
        *lock(&self.session) = Some(session.clone());
        self.session_out.send_replace(Some(session.clone()));
        self.bots.set_session(Some(session.clone()));
        self.apply_pending_display_name(&session);
        self.view.send_modify(|view| {
            view.account = ChatAccount::SignedIn {
                user_id: session.user_id().to_string(),
                device_id: session.device_id().to_string(),
            };
            view.sync = ChatSync::Starting;
        });
        let tasks = self.observe(&session);
        *active = Some(Active { tasks });
        drop(active);
        if let Err(error) = session.start_sync().await {
            self.view.send_modify(|view| {
                view.sync = ChatSync::Failed {
                    reason: failure(&error),
                    message: error.to_string(),
                };
            });
        }
    }

    /// 暂时的原因就显示“正在重连”，等一会儿再登录；否则停在失败上，等用户处理（回到前台时也会再试一次）。
    fn sign_in_failed(
        self: Arc<Self>,
        generation: u64,
        target: Target,
        reason: ChatFailure,
        message: String,
    ) {
        // 在锁里改显示：判断完代数到改完之间，新的一次登录插不进来改成“正在登录”又被这里盖掉。
        let delay = {
            let mut relogin = lock(&self.relogin.0);
            // 这期间又开始了一次登录，由那一次决定。
            if relogin.generation != generation {
                return;
            }
            let delay = if temporary(reason) {
                relogin.failures += 1;
                Some(backoff(relogin.failures))
            } else {
                relogin.failures = 0;
                None
            };
            let account = match delay {
                Some(_) => ChatAccount::Reconnecting { reason, message },
                None => ChatAccount::Failed { reason, message },
            };
            self.view.send_modify(|view| {
                view.account = account;
                view.sync = ChatSync::Idle;
            });
            delay
        };
        let Some(delay) = delay else { return };
        tracing::info!(?delay, "will sign in to Matrix again");
        // 等的时候不占着作用域：换了服务器，旧作用域照常释放，醒来升级失败就作罢。
        let chat = Arc::downgrade(&self);
        self.runtime.spawn(async move {
            tokio::time::sleep(delay).await;
            let Some(chat) = chat.upgrade() else { return };
            if lock(&chat.relogin.0).generation == generation {
                chat.connect(target).await;
            }
        });
    }

    /// 家里的连接没了：停掉同步，留着登录状态，下次连上时恢复。
    pub(super) async fn disconnect(&self) {
        lock(&self.relogin.0).generation += 1;
        let mut active = self.active.lock().await;
        if active.take().is_some() {
            self.forget_current().await;
        }
        *lock(&self.signing_proxy) = None;
        self.view.send_modify(|view| {
            view.account = ChatAccount::SignedOut;
            view.sync = ChatSync::Idle;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 行为：对方公网关着会自己好，按暂时的原因退避重登，不停在失败上等用户。
    #[test]
    fn peer_public_access_off_is_temporary() {
        assert!(temporary(ChatFailure::PeerPublicAccessOff));
    }

    /// 行为：对方服务器没配对要先配对，重登也到不了，停在失败上。
    #[test]
    fn server_not_paired_is_not_temporary() {
        assert!(!temporary(ChatFailure::ServerNotPaired));
    }
}
