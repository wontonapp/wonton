//! Matrix 会话的事件：令牌刷新后存下，设备被吊销时如实显示并删掉用不上的登录状态和本地存储。

use std::sync::{Arc, Weak};

use futures_util::StreamExt;
use wonton_l10n::l10n;
use wonton_matrix::{MatrixSession, SessionEvent};

use super::ChatLink;
use crate::l10n::{CATALOG, language};

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
                    tracing::warn!("could not save the refreshed Matrix tokens: {error}");
                }
            }
            SessionEvent::TokenRejected { soft_logout } => {
                tracing::warn!(
                    soft_logout,
                    "the Matrix server no longer accepts this device's tokens"
                );
                chat.bots.set_session(None);
                chat.session_out.send_replace(None);
                {
                    let mut open = super::lock(&chat.open);
                    open.clear();
                    chat.room_panel_out.send_replace(None);
                }
                if !soft_logout {
                    session.stop_sync().await;
                    if let Err(error) = chat.storage.forget_session() {
                        tracing::warn!("could not delete the invalid Matrix login state: {error}");
                    }
                }
                chat.view.send_modify(|view| {
                    view.account = wonton_protocol::ChatAccount::Failed {
                        reason: wonton_protocol::ChatFailure::Unauthorized,
                        message: l10n!(CATALOG, language(), "chat-session-device-revoked"),
                    };
                    view.sync = wonton_protocol::ChatSync::Idle;
                });
            }
        }
    }
}
