//! `ChatLink` 往外递的通道，以及卡片成功钩子的两种编译形态。

use std::sync::Arc;

#[cfg(feature = "test-hooks")]
use futures_util::future::BoxFuture;
use tokio::sync::watch;
use wonton_matrix::MatrixSession;
use wonton_protocol::{ChatViewModel, ConfirmationsViewModel, PushMethod};

/// 卡片变成成功时就地调用（插件 ID），在卡片的情况发出去之前：ViewModel 里卡片“已经不用回答”时，补不补取已经定了。
#[cfg(not(feature = "test-hooks"))]
pub(crate) type CardSuccess = Arc<dyn Fn(&str) + Send + Sync>;

/// 开着测试钩子（feature `test-hooks`）时，决定是个可等待的 future：钩子先报到、等放行，决定才继续，
/// settled 也发不出去。见 `test_hooks`。
#[cfg(feature = "test-hooks")]
pub(crate) type CardSuccess = Arc<dyn for<'a> Fn(&'a str) -> BoxFuture<'a, ()> + Send + Sync>;

/// ChatLink 对外的通道：scope 建好，chat 往里喂，tabs 等拿去用。
pub(crate) struct ChatChannels {
    /// ViewModel 发送端，换服务器时给壳接着订阅用。
    pub(crate) view: Option<watch::Sender<ChatViewModel>>,
    /// “等你确认”列表的发送端，同上。
    pub(crate) confirmations: Option<watch::Sender<ConfirmationsViewModel>>,
    /// 推送方式。
    pub(crate) method: watch::Receiver<PushMethod>,
    /// App 锁，锁着时通知藏内容。
    pub(crate) app_lock: crate::app_lock::AppLockLink,
    /// 锁屏上隐藏内容的类别（ADR-0101 §5），通知照它设系统的可见性。
    pub(crate) lock_screen: watch::Receiver<crate::lock_screen::Hidden>,
    /// 登录着的会话。
    pub(crate) session_out: watch::Sender<Option<Arc<MatrixSession>>>,
    /// 外部 Matrix 账号的槽位（ADR-0051）：不跟着家里的连接走，直连 homeserver，用登录时存下的状态恢复。
    pub(crate) external: bool,
    /// 推送里带来的正文预览（ADR-0118）：推送写、时间线读，同一个实例。
    pub(crate) push_previews: Arc<crate::push::PushPreviews>,
}
