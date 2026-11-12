//! Matrix 会话的事件：令牌刷新后存下，设备被吊销时如实显示并删掉用不上的登录状态和本地存储。

use std::sync::{Arc, Weak};

use futures_util::StreamExt;
use wonton_matrix::{MatrixSession, SessionEvent};

use super::ChatLink;

pub(super) async fn watch(chat: Weak<ChatLink>, session: Arc<MatrixSession>) {
    let mut events = std::pin::pin!(session.session_events());
    while let Some(event) = events.next().await {
        let Some(chat) = chat.upgrade() else { return };
        match event {
            SessionEvent::TokensRefreshed => {
                let saved = chat.storage.session().and_then(|stored| match stored {
                    Some(stored) => chat.save(&session, stored.homeserver_url, stored.server_id),
                    None => Ok(()),
                });
                if let Err(error) = saved {
                    tracing::warn!("刷新后的 Matrix 令牌没能存下：{error}");
                }
            }
            SessionEvent::TokenRejected { soft_logout } => {
                tracing::warn!(soft_logout, "Matrix 服务器不再接受本设备的令牌");
                if !soft_logout {
                    session.stop_sync().await;
                    if let Err(error) = chat.storage.forget_session() {
                        tracing::warn!("删除失效的 Matrix 登录状态失败：{error}");
                    }
                }
                chat.view.send_modify(|view| {
                    view.account = wonton_protocol::ChatAccount::Failed {
                        reason: wonton_protocol::ChatFailure::Unauthorized,
                        message: "这台设备在 Matrix 上已被吊销，需要主人重新配对".to_owned(),
                    };
                    view.sync = wonton_protocol::ChatSync::Idle;
                });
            }
        }
    }
}
