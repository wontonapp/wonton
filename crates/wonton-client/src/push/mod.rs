//! 推送（ADR-0003 的“设计：加密推送”）：把平台给的推送订阅经隧道登记给家里的核心，再在 Matrix 上注册指向家里
//! 推送代理的 pusher；收到推送时取回那条消息、在本机解密（见 `notification`）。
//!
//! 登记跟着聊天走：每次登录（包括从本机恢复会话）后都重新登记一遍，pusher 被 Tuwunel 删掉过（推送代理回过
//! `rejected`）也能补回来。推送订阅不存盘：壳每次启动都从分发器重新拿到它，推送唤醒的进程用不着它。

mod notification;
mod registration;

use std::sync::{Arc, Mutex, MutexGuard, Weak};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use tokio::runtime::Handle;
use tokio::sync::watch;
use wonton_matrix::PusherConfig;
use wonton_protocol::{
    ChatAccount, ChatFailure, ChatViewModel, CoreError, PushEndpoint, PushStatus,
};

use crate::chat::{ChatLink, failure};
use crate::home::HomeLink;

/// Matrix pusher 的 `app_id`。推送代理按登记找设备，不靠它区分平台，所以各端共用一个。
const APP_ID: &str = "app.wonton";
const APP_NAME: &str = "Wonton";
/// 本设备在 pusher 列表里的名字，和 Matrix 设备名一致。
const DEVICE_NAME: &str = "Wonton App";
const LANG: &str = "zh-CN";

pub(crate) struct PushLink {
    runtime: Handle,
    home: Arc<HomeLink>,
    chat: Arc<ChatLink>,
    state: Mutex<State>,
    /// 串行化登记和取消登记。
    busy: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct State {
    endpoint: Option<PushEndpoint>,
    /// 最近一次登记拿到的 pushkey，取消登记时删 pusher 用。
    pushkey: Option<String>,
}

impl PushLink {
    pub(crate) fn new(runtime: Handle, home: Arc<HomeLink>, chat: Arc<ChatLink>) -> Arc<Self> {
        let push = Arc::new(Self {
            runtime: runtime.clone(),
            home,
            chat: chat.clone(),
            state: Mutex::new(State::default()),
            busy: tokio::sync::Mutex::new(()),
        });
        runtime.spawn(follow_chat(Arc::downgrade(&push), chat.subscribe()));
        push
    }

    /// `SetPushEndpoint`：校验后记下，登录着就马上登记，没登录的话登录后登记。
    pub(crate) fn set_endpoint(self: &Arc<Self>, endpoint: PushEndpoint) -> Result<(), CoreError> {
        validate(&endpoint)?;
        self.lock().endpoint = Some(endpoint);
        self.chat.set_push(PushStatus::Registering);
        self.runtime.spawn(self.clone().register());
        Ok(())
    }

    /// `PushUnavailable`：如实显示，删掉家里的登记和 pusher。尽力而为：分发器已经不送了，留下的登记只会白白推送；
    /// 删不掉的 pusher，推送代理下次会回 `rejected`，Tuwunel 随之删掉它。
    pub(crate) fn unavailable(self: &Arc<Self>, message: String) {
        self.lock().endpoint = None;
        self.chat.set_push(PushStatus::Unavailable { message });
        self.runtime.spawn(self.clone().unregister());
    }

    /// 回到前台：上次登记失败（不是被吊销）的话再试一次。
    pub(crate) fn foregrounded(self: &Arc<Self>) {
        let retry = matches!(
            self.chat.view().push,
            PushStatus::Failed { reason, .. } if reason != ChatFailure::Unauthorized
        );
        if retry {
            self.runtime.spawn(self.clone().register());
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 登记给家里的核心、注册 pusher。登记期间推送订阅又换了的话，用新的再来一遍。
    async fn register(self: Arc<Self>) {
        let _busy = self.busy.lock().await;
        loop {
            let Some(endpoint) = self.lock().endpoint.clone() else {
                return;
            };
            // 还没登录：状态留在“正在登记”，登录后 `follow_chat` 会再来。
            let Ok(session) = self.chat.current() else {
                return;
            };
            let (Some(proxy), Some(core_url)) = (self.home.view().proxy, self.home.core_url())
            else {
                return;
            };
            self.chat.set_push(PushStatus::Registering);
            let registered = async {
                let registration = registration::register(&core_url, &proxy, &endpoint).await?;
                let pusher = PusherConfig {
                    app_id: APP_ID.to_owned(),
                    pushkey: registration.pushkey.clone(),
                    gateway_url: registration.gateway_url,
                    app_display_name: APP_NAME.to_owned(),
                    device_display_name: DEVICE_NAME.to_owned(),
                    lang: LANG.to_owned(),
                };
                session
                    .set_pusher(&pusher)
                    .await
                    .map_err(|e| (failure(&e), format!("在 Matrix 上注册 pusher 失败：{e}")))?;
                Ok::<_, (ChatFailure, String)>(registration.pushkey)
            }
            .await;
            let mut state = self.lock();
            if state.endpoint.as_ref() != Some(&endpoint) {
                continue;
            }
            match registered {
                Ok(pushkey) => {
                    state.pushkey = Some(pushkey);
                    drop(state);
                    tracing::info!("推送已登记");
                    self.chat.set_push(PushStatus::Registered);
                }
                Err((reason, message)) => {
                    drop(state);
                    tracing::info!("推送登记失败：{message}");
                    self.chat.set_push(PushStatus::Failed { reason, message });
                }
            }
            return;
        }
    }

    /// 家里的登记按设备删，不用 pushkey，上次启动时留下的也删得掉；pusher 只删这次登记过的。App 刚启动就收不了推送
    /// 时（比如主人拒绝了通知权限），先等会话就绪。
    async fn unregister(self: Arc<Self>) {
        let _busy = self.busy.lock().await;
        let session = match self.wait_session().await {
            Ok(session) => session,
            Err((_, message)) => {
                tracing::info!("没能取消推送登记：{message}");
                return;
            }
        };
        if self.lock().endpoint.is_some() {
            return;
        }
        if let (Some(proxy), Some(core_url)) = (self.home.view().proxy, self.home.core_url())
            && let Err((_, message)) = registration::unregister(&core_url, &proxy).await
        {
            tracing::info!("{message}");
        }
        let pushkey = self.lock().pushkey.take();
        if let Some(pushkey) = pushkey
            && let Err(error) = session.delete_pusher(APP_ID, &pushkey).await
        {
            tracing::info!("删除 pusher 失败：{error}");
        }
    }
}

/// 每次登录（换了设备 ID 也算）后重新登记。
async fn follow_chat(push: Weak<PushLink>, mut chat: watch::Receiver<ChatViewModel>) {
    let mut device = None;
    loop {
        let signed_in = match &chat.borrow_and_update().account {
            ChatAccount::SignedIn { device_id, .. } => Some(device_id.clone()),
            _ => None,
        };
        let Some(link) = push.upgrade() else { return };
        if signed_in.is_some() && signed_in != device {
            link.runtime.spawn(link.clone().register());
        }
        device = signed_in;
        drop(link);
        if chat.changed().await.is_err() {
            return;
        }
    }
}

/// 推送地址是 http 或 https；`p256dh` 是 65 字节的未压缩 P-256 点，`auth` 是 16 字节，都是 base64url。曲线上的点
/// 由家里的核心再校验一次。
fn validate(endpoint: &PushEndpoint) -> Result<(), CoreError> {
    let invalid = |reason: &str| CoreError::InvalidInput {
        reason: reason.to_owned(),
    };
    let url = url::Url::parse(&endpoint.url)
        .map_err(|e| invalid(&format!("推送地址不是合法的 URL：{e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid("推送地址必须是 http 或 https 地址"));
    }
    let decode = |value: &str| URL_SAFE_NO_PAD.decode(value.trim_end_matches('='));
    match decode(&endpoint.p256dh) {
        Ok(key) if key.len() == 65 && key[0] == 0x04 => {}
        _ => {
            return Err(invalid(
                "p256dh 应为 65 字节的未压缩 P-256 公钥（base64url）",
            ));
        }
    }
    match decode(endpoint.auth.expose()) {
        Ok(auth) if auth.len() == 16 => Ok(()),
        _ => Err(invalid("auth 应为 16 字节（base64url）")),
    }
}
