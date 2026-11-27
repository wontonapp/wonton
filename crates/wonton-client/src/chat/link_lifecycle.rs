//! 断开时的清理（`forget_current`）和会话的观察任务（`observe`）：后台通知、房间列表、布局、加密和同步事件。

use std::sync::Arc;

use futures_util::StreamExt;
use tokio::task::JoinHandle;
use wonton_matrix::{MatrixSession, SyncStatus};
use wonton_protocol::{ChatFailure, ChatSync};

use super::ChatLink;
use super::failure;
use super::layout;
use super::recovery;
use super::session;
use super::util::lock;

/// 换会话时最多等旧同步收尾这么久；死连接的长轮询一轮 30 秒，等不到就直接放掉旧会话。
const STOP_SYNC_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

impl ChatLink {
    pub(super) async fn forget_current(&self) {
        self.bots.set_session(None);
        self.session_out.send_replace(None);
        lock(&self.open).clear();
        *lock(&self.verification) = None;
        self.side_chat_support.clear();
        self.room_order.clear();
        self.sent.send_replace(Vec::new());
        self.home.set_catching_up(false);
        self.view.send_modify(|view| {
            recovery::forgot_session(&mut view.encryption);
            // 确认卡片跟家里的连接走，不和 Matrix 会话绑定：这里不清。
        });
        let session = lock(&self.session).take();
        if let Some(session) = session {
            // `stop_sync` 会等 `sync_service` 的 supervisor 和两条同步任务收完。`sliding_sync::sync` 的
            // `select!` 是 `biased` 的：`SyncLoopStop` 一到就把正在跑的 `sync_once` 连请求一起丢掉；收不了场的是
            // 拿到应答之后的处理（`handle_response` + 写存储），隧道刚断、代理已经不在了的时候可能就是这一轮。
            //
            // 所以只等一小会儿就放掉旧会话：Arc 一落，会话和它的 `Client` 跟着落，新登录不用排在这轮处理后面。
            // 超时的那条路上同步服务的任务还在跑（它们自己持有 `Client`），存储租约要替名字多活一会儿，见下。
            if tokio::time::timeout(STOP_SYNC_GRACE, session.stop_sync())
                .await
                .is_err()
            {
                tracing::info!(slot = %self.slot, "旧同步没停下来，直接放掉旧会话");
                let session = session.clone();
                self.runtime
                    .spawn(async move { session.keep_store_lease_until_sync_settles().await });
            }
        }
    }

    /// 把会话的房间列表、加密状态、同步状态和会话事件接到 ViewModel 上。
    pub(super) fn observe(self: &Arc<Self>, session: &Arc<MatrixSession>) -> Vec<JoinHandle<()>> {
        let view = self.view.clone();
        let background = self.observe_background(session);
        let rooms = self.observe_rooms(session);
        let layouts = layout::follow(self, session).expect("the chat link is still alive here");
        let encryption = self
            .runtime
            .spawn(recovery::watch(Arc::downgrade(self), session.clone()));
        let sync = {
            let statuses = session.sync_status();
            let view = view.clone();
            let chat = Arc::downgrade(self);
            self.runtime.spawn(async move {
                let mut statuses = std::pin::pin!(statuses);
                while let Some(status) = statuses.next().await {
                    // 会话还挂在一个已经拆掉的代理上：同步自己重启多少次都只会再失败。换成当前代理重新
                    // 登录一次（和 App 回到前台走同一条路），新会话用新连接，通知和消息才接得上。
                    if let SyncStatus::Failed(error) = &status
                        && failure(error) == ChatFailure::TransportUnavailable
                        && let Some(link) = chat.upgrade()
                        // 外部 Matrix 账号走 `External`，`signing_proxy` 恒为 None，别改登家里的服务器。
                        && !link.external
                        && let Some(proxy) = link.home.view().proxy.clone()
                        && lock(&link.signing_proxy).as_ref() != Some(&proxy)
                    {
                        link.runtime
                            .spawn(link.clone().connect(super::connect::Target::Home(proxy)));
                    }
                    let sync = match status {
                        SyncStatus::Idle => ChatSync::Idle,
                        SyncStatus::Starting => ChatSync::Starting,
                        SyncStatus::Running => ChatSync::Running,
                        SyncStatus::Failed(error) => ChatSync::Failed {
                            reason: failure(&error),
                            message: error.to_string(),
                        },
                    };
                    view.send_if_modified(|view| {
                        // 刚登录时 SDK 的同步服务还没启动，它报的“未启动”不覆盖“正在启动”。
                        if sync == ChatSync::Idle && view.sync == ChatSync::Starting {
                            return false;
                        }
                        view.sync = sync;
                        true
                    });
                }
            })
        };
        let events = self
            .runtime
            .spawn(session::watch(Arc::downgrade(self), session.clone()));
        let syncing = {
            let session = session.clone();
            self.runtime
                .spawn(async move { session.keep_syncing().await })
        };
        let catching_up = {
            let updates = session.catching_up();
            let home = self.home.clone();
            self.runtime.spawn(async move {
                let mut updates = std::pin::pin!(updates);
                while let Some(catching_up) = updates.next().await {
                    home.set_catching_up(catching_up);
                }
            })
        };
        let mut tasks = vec![
            background,
            rooms,
            layouts,
            encryption,
            sync,
            events,
            syncing,
            catching_up,
        ];
        tasks.extend(self.verification_tasks(session));
        tasks
    }
}
