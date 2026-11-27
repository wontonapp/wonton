//! 断开时的清理（`forget_current`）和会话的观察任务（`observe`）：后台通知、房间列表、布局、加密和同步事件。

use std::sync::Arc;

use futures_util::StreamExt;
use tokio::task::JoinHandle;
use wonton_matrix::{MatrixSession, SyncStatus};
use wonton_protocol::ChatSync;

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
            // 停同步要等滑动同步在途的那一次请求回来：matrix-sdk 的 sliding sync 只在两次请求之间收
            // `SyncLoopStop`（`sliding_sync::sync` 的 select 里，请求在 `sync_once` 内不受打断）。隧道刚断、
            // 代理已经不在了的时候，那次长轮询要等到自己超时，几十秒都有可能；而调用方这时多半正要重新登录，
            // 等待期间 `session` 是空的，要会话的动作全吃 `NotReady`。
            //
            // 所以只等一小会儿就放掉旧会话：Arc 一落，客户端也就走了，它在途的那次请求跟着断，服务器那边的
            // 长轮询随之了断。把旧的留在后台停、让新登录和它并存，反而会和它的在途请求抢同一个会话（实测
            // `a_restart_does_not_wait_for_the_dead_long_poll` 从 12s 变成 40s）。
            if tokio::time::timeout(STOP_SYNC_GRACE, session.stop_sync())
                .await
                .is_err()
            {
                tracing::info!(slot = %self.slot, "旧同步没停下来，直接放掉旧会话");
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
            self.runtime.spawn(async move {
                let mut statuses = std::pin::pin!(statuses);
                while let Some(status) = statuses.next().await {
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
