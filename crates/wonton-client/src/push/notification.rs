//! 收到推送：解析家里推送代理发来的明文，等 Matrix 会话就绪，取回那条消息、在本机解密，给出要显示的通知。
//!
//! 推送唤醒的进程刚起来：隧道要连上，会话要从本机恢复。取不到正文时照实给出原因，不显示成正常的消息。

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Map, Value};
use wonton_matrix::{
    EventId, MatrixSession, MessageBody, Notification, OwnedEventId, OwnedRoomId, RoomId, UserId,
};
use wonton_protocol::{
    ChatAccount, ChatFailure, CoreError, HomeStatus, MessageSummary, NOTICE_FIELD,
    NotificationCategory, PluginNotice, PushContent, PushNotification,
};

use super::PushLink;
use crate::chat::{PushedCard, core_account, failure, pushed_card, summary};

/// 等 Matrix 会话就绪最多这么久：推送唤醒的进程要先建好本地代理、从本机恢复会话。
const SESSION_WAIT: Duration = Duration::from_secs(12);

/// 取消息最多等这么久。matrix-sdk 的通知客户端有自己的超时（缺房间密钥时最多再等几秒），这里兜底，隧道卡住时
/// 也能在系统给的后台时间里给出结果。
const FETCH_WAIT: Duration = Duration::from_secs(15);

/// 推送解开后的明文，字段同核心的 `DevicePayload`（核心有契约测试锁住字段名）。
#[derive(Clone, Deserialize)]
pub(crate) struct Payload {
    /// 发推送的服务器的 endpoint ID。
    pub(crate) server: String,
    room_id: String,
    event_id: String,
    sender: Option<String>,
    sender_display_name: Option<String>,
    room_name: Option<String>,
}

impl Payload {
    /// 不是家里推送代理发的格式时返回 `InvalidInput`。
    pub(crate) fn parse(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|e| invalid(&format!("推送内容的格式不对：{e}")))
    }

    /// 取不到正文时的通知：只有推送里带的发送者和房间，原因照实写。
    pub(crate) fn unavailable(&self, reason: ChatFailure, message: String) -> PushNotification {
        PushNotification {
            // 槽位由 `ClientCore` 填：推送这一层不知道自己属于哪个槽位。
            server_id: None,
            room_id: self.room_id.clone(),
            event_id: self.event_id.clone(),
            sender: self
                .sender_display_name
                .clone()
                .or_else(|| self.sender.clone())
                .unwrap_or_else(|| "未知的发送者".to_owned()),
            room_name: self.room_name.clone(),
            content: PushContent::Unavailable { reason, message },
            category: NotificationCategory::Message,
            confirmation: None,
        }
    }
}

impl PushLink {
    /// 不用显示（按推送规则不该响、消息已撤回）时返回 `None`。
    pub(crate) async fn resolve(
        &self,
        payload: &Payload,
    ) -> Result<Option<PushNotification>, CoreError> {
        let started = Instant::now();
        let room_id: OwnedRoomId = payload
            .room_id
            .parse()
            .map_err(|e| invalid(&format!("推送里的房间 ID 不合法：{e}")))?;
        let event_id: OwnedEventId = payload
            .event_id
            .parse()
            .map_err(|e| invalid(&format!("推送里的事件 ID 不合法：{e}")))?;
        let unavailable = |reason, message| payload.unavailable(reason, message);
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
        Ok(push_notification(
            None,
            session.user_id(),
            &room_id,
            &event_id,
            message,
        ))
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

/// 推送和保持连接的后台通知共用的映射（ADR-0003 的“设计：保持连接”）：发送者、房间名、类别、确认卡片、
/// 取不到正文的占位。卡片已经过期时不用通知，返回 `None`。
pub(crate) fn push_notification(
    server_id: Option<String>,
    own_user: &UserId,
    room_id: &RoomId,
    event_id: &EventId,
    message: Notification,
) -> Option<PushNotification> {
    let confirmation = match pushed_card(own_user, &message.sender, &message.custom) {
        PushedCard::None => None,
        PushedCard::Waiting(card) => Some(card),
        PushedCard::Expired => return None,
    };
    let notice = plugin_notice(own_user, &message.sender, &message.custom);
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
    Some(PushNotification {
        server_id,
        room_id: room_id.to_string(),
        event_id: event_id.to_string(),
        sender: message.sender_name,
        room_name: message.room_name,
        content,
        category: if confirmation.is_some() {
            NotificationCategory::Confirmation
        } else if notice.is_some() {
            NotificationCategory::PluginNotice
        } else {
            NotificationCategory::Message
        },
        confirmation,
    })
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

/// 核心账号发的、带插件通知字段且读得懂的消息（ADR-0047）。别人发的同样结构的内容当普通消息；比这个 App 新的核心
/// 写的、读不懂的也是：`body` 是写给其他客户端看的纯文本，主人看得到。
fn plugin_notice(
    own_user: &UserId,
    sender: &UserId,
    custom: &Map<String, Value>,
) -> Option<PluginNotice> {
    if sender != core_account(own_user) {
        return None;
    }
    serde_json::from_value(custom.get(NOTICE_FIELD)?.clone()).ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wonton_matrix::OwnedUserId;

    use super::*;

    fn custom(field: Value) -> Map<String, Value> {
        [(NOTICE_FIELD.to_owned(), field)].into_iter().collect()
    }

    fn field() -> Value {
        json!({
            "plugin_id": "app.example.monitor",
            "plugin_name": "Monitor",
            "level": "warning",
            "title": "磁盘快满了",
        })
    }

    #[test]
    fn only_the_core_accounts_structured_messages_are_plugin_notices() {
        let owner: OwnedUserId = "@owner:test".parse().unwrap();
        let core: OwnedUserId = "@wonton-core:test".parse().unwrap();
        let stranger: OwnedUserId = "@stranger:test".parse().unwrap();
        assert!(plugin_notice(&owner, &core, &custom(field())).is_some());
        // 别人发的同样结构当普通消息。
        assert!(plugin_notice(&owner, &stranger, &custom(field())).is_none());
        // 别的服务器上同名账号发的也不是。
        let other_server: OwnedUserId = "@wonton-core:other.example".parse().unwrap();
        assert!(plugin_notice(&owner, &other_server, &custom(field())).is_none());
        // 核心发的、读不懂的当普通消息。
        assert!(plugin_notice(&owner, &core, &custom(json!({"level": "warning"}))).is_none());
        // 没有这个字段的是普通消息。
        assert!(plugin_notice(&owner, &core, &Map::new()).is_none());
    }
}
