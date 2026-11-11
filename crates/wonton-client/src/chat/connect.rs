//! 登录、断开，以及登录因为暂时的原因没成功时按退避自动重登。
//!
//! 家里的连接一直连着、只是那一次登录请求失败（满载时要令牌超时、网关回 502、Matrix 还在启动）时，`follow_home` 看不到
//! 任何变化，不会再登录；不自己重试的话聊天就一直断着，直到 App 回到前台。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use wonton_protocol::{ChatAccount, ChatFailure, ChatSync, ProxySettings};

use super::{Active, ChatLink, failure, lock};

/// 登录到哪里：经家里的本地代理登录配对的服务器，或者直连外部 Matrix 账号的 homeserver（ADR-0051）。
#[derive(Clone, PartialEq, Eq)]
pub(super) enum Target {
    Home(ProxySettings),
    External,
}

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

/// 连不上、Matrix 还没就绪会自己好；被拒、输入非法、服务器出错要用户处理，重试也没用。
fn temporary(reason: ChatFailure) -> bool {
    matches!(
        reason,
        ChatFailure::TransportUnavailable | ChatFailure::NotReady
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
        // 等锁时可能又换了连接：这个代理已经没了，不能凭它动下一台服务器的登录状态。外部账号的槽位一个槽位一个账号，
        // 账号数据按槽位记。
        let server_id = match &target {
            Target::Home(proxy) => {
                let home = self.home.view();
                let Some(server_id) = home
                    .server_id
                    .filter(|_| home.proxy.as_ref() == Some(proxy))
                else {
                    return;
                };
                server_id
            }
            Target::External => String::new(),
        };
        if let Some(old) = active.take() {
            self.forget_current().await;
            drop(old);
        }
        // 账号数据跟着服务器走：断线时先留着，换服务器才清。
        if self.layouts.adopt(&server_id) {
            self.view
                .send_modify(|view| view.chat_layout_default = Default::default());
        }
        self.view
            .send_modify(|view| view.account = ChatAccount::SigningIn);
        let signed_in = match &target {
            Target::Home(proxy) => self.sign_in(proxy, server_id).await,
            Target::External => self.sign_in_external().await,
        };
        let session = match signed_in {
            Ok(session) => Arc::new(session),
            Err((reason, message)) => {
                tracing::info!("Matrix sign-in failed: {message}");
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
        self.view.send_modify(|view| {
            view.account = ChatAccount::SignedOut;
            view.sync = ChatSync::Idle;
        });
    }
}
