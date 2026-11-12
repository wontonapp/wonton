//! 收到推送：解析家里推送代理发来的明文，等 Matrix 会话就绪，取回那条消息、在本机解密，给出要显示的通知。
//!
//! 推送唤醒的进程刚起来：隧道要连上，会话要从本机恢复。取不到正文时照实给出原因，不显示成正常的消息。

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use wonton_matrix::{MatrixSession, MessageBody, OwnedEventId, OwnedRoomId};
use wonton_protocol::{
    ChatAccount, ChatFailure, CoreError, HomeStatus, MessageSummary, NotificationCategory,
    PushContent, PushNotification,
};

use super::PushLink;
use crate::chat::{PushedCard, failure, pushed_card, summary};

/// 等 Matrix 会话就绪最多这么久：推送唤醒的进程要先建好本地代理、从本机恢复会话。
const SESSION_WAIT: Duration = Duration::from_secs(12);

/// 取消息最多等这么久。matrix-sdk 的通知客户端有自己的超时（缺房间密钥时最多再等几秒），这里兜底，隧道卡住时
/// 也能在系统给的后台时间里给出结果。
const FETCH_WAIT: Duration = Duration::from_secs(15);

/// 推送解开后的明文，字段同核心的 `DevicePayload`（核心有契约测试锁住字段名）。
#[derive(Deserialize)]
struct Payload {
    room_id: String,
    event_id: String,
    sender: Option<String>,
    sender_display_name: Option<String>,
    room_name: Option<String>,
}

impl PushLink {
    /// 推送内容不是家里推送代理发的格式时返回 `InvalidInput`；不用显示（按推送规则不该响、消息已撤回）时返回 `None`。
    pub(crate) async fn resolve(
        &self,
        payload: &[u8],
    ) -> Result<Option<PushNotification>, CoreError> {
        let started = Instant::now();
        let payload: Payload = serde_json::from_slice(payload)
            .map_err(|e| invalid(&format!("推送内容的格式不对：{e}")))?;
        let room_id: OwnedRoomId = payload
            .room_id
            .parse()
            .map_err(|e| invalid(&format!("推送里的房间 ID 不合法：{e}")))?;
        let event_id: OwnedEventId = payload
            .event_id
            .parse()
            .map_err(|e| invalid(&format!("推送里的事件 ID 不合法：{e}")))?;
        let server_id = self.home.view().server_id;
        let notification = |content| PushNotification {
            server_id: server_id.clone(),
            room_id: payload.room_id.clone(),
            event_id: payload.event_id.clone(),
            sender: payload
                .sender_display_name
                .clone()
                .or_else(|| payload.sender.clone())
                .unwrap_or_else(|| "未知的发送者".to_owned()),
            room_name: payload.room_name.clone(),
            content,
            category: NotificationCategory::Message,
            confirmation: None,
        };
        let unavailable =
            |reason, message| notification(PushContent::Unavailable { reason, message });
        let session = match self.wait_session().await {
            Ok(session) => session,
            Err((reason, message)) => {
                tracing::info!(elapsed = ?started.elapsed(), %message, "推送：会话没就绪");
                return Ok(Some(unavailable(reason, message)));
            }
        };
        let session_ready = started.elapsed();
        let fetched =
            tokio::time::timeout(FETCH_WAIT, session.notification(&room_id, &event_id)).await;
        tracing::info!(?session_ready, elapsed = ?started.elapsed(), "推送：取消息结束");
        let message = match fetched {
            Err(_) => {
                let message = format!("{} 秒内没能取回这条消息", FETCH_WAIT.as_secs());
                return Ok(Some(unavailable(
                    ChatFailure::TransportUnavailable,
                    message,
                )));
            }
            Ok(Err(error)) => {
                let message = format!("取不到这条消息：{error}");
                return Ok(Some(unavailable(failure(&error), message)));
            }
            Ok(Ok(None)) => return Ok(None),
            Ok(Ok(Some(message))) => message,
        };
        let confirmation = match pushed_card(session.user_id(), &message.sender, &message.custom) {
            PushedCard::None => None,
            PushedCard::Waiting(card) => Some(card),
            PushedCard::Expired => return Ok(None),
        };
        let content = match message.body {
            MessageBody::Undecryptable => PushContent::Unavailable {
                reason: ChatFailure::NotReady,
                message: "还没收到这条消息的房间密钥".to_owned(),
            },
            MessageBody::Other { kind } if kind == "m.room.member" => PushContent::Message {
                body: "邀请你加入".to_owned(),
            },
            body => PushContent::Message {
                body: push_text(summary(body, false)),
            },
        };
        Ok(Some(PushNotification {
            sender: message.sender_name,
            room_name: message.room_name,
            category: if confirmation.is_some() {
                NotificationCategory::Confirmation
            } else {
                NotificationCategory::Message
            },
            confirmation,
            ..notification(content)
        }))
    }

    /// 等聊天登录（或从本机恢复）好。登录失败、超时都给出原因。
    pub(super) async fn wait_session(&self) -> Result<Arc<MatrixSession>, (ChatFailure, String)> {
        let mut chat = self.chat.subscribe();
        let waiting = async {
            loop {
                if let Ok(session) = self.chat.current() {
                    return Ok(session);
                }
                if let ChatAccount::Failed { reason, message } = &chat.borrow_and_update().account {
                    return Err((*reason, message.clone()));
                }
                if chat.changed().await.is_err() {
                    return Err((ChatFailure::Upstream, "客户端核心已关闭".to_owned()));
                }
            }
        };
        match tokio::time::timeout(SESSION_WAIT, waiting).await {
            Ok(result) => result,
            Err(_) => Err(match self.home.view().status {
                HomeStatus::NotConfigured => {
                    (ChatFailure::NotReady, "还没有连过家里的服务器".to_owned())
                }
                HomeStatus::Disconnected { message, .. } => (
                    ChatFailure::TransportUnavailable,
                    format!("连不上家里的服务器：{message}"),
                ),
                HomeStatus::Connecting | HomeStatus::Connected => (
                    ChatFailure::TransportUnavailable,
                    format!("{} 秒内没能登录 Matrix", SESSION_WAIT.as_secs()),
                ),
            }),
        }
    }
}

/// 推送的正文是字符串：这里把摘要转成占位文字，图片、文件等的占位写在这里。
fn push_text(summary: MessageSummary) -> String {
    match summary {
        MessageSummary::Text { text } => text,
        MessageSummary::Files => "[文件]".to_owned(),
        MessageSummary::Image => "[图片]".to_owned(),
        MessageSummary::Undecryptable => "[加密消息]".to_owned(),
        MessageSummary::Redacted => "[已撤回]".to_owned(),
        MessageSummary::Other { kind } => format!("[{kind}]"),
    }
}

fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidInput {
        reason: reason.to_owned(),
    }
}
