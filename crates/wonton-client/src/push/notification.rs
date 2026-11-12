//! 收到推送：解析家里推送代理发来的明文，等 Matrix 会话就绪，取回那条消息、在本机解密，给出要显示的通知。
//!
//! 推送唤醒的进程刚起来：隧道要连上，会话要从本机恢复。取不到正文时照实给出原因，不显示成正常的消息。

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Map, Value};
use url::Url;
use wonton_l10n::l10n;
use wonton_matrix::{
    EventId, MatrixSession, MessageBody, Notification, OwnedEventId, OwnedRoomId, RoomId, UserId,
};
use wonton_protocol::{
    ChatAccount, ChatFailure, ConfirmationState, ContactRequestPush, CoreError,
    DeviceConfirmationPush, HomeStatus, MessageSummary, NOTICE_FIELD, NotificationCategory,
    PluginNotice, ProxySettings, PushConfirmation, PushContent, PushNotification,
};

use super::PushLink;
use crate::chat::{LinkTargets, core_account, failure, fetch_card, summary};
use crate::l10n::{CATALOG, language};

/// 等 Matrix 会话就绪最多这么久：推送唤醒的进程要先建好本地代理、从本机恢复会话。
const SESSION_WAIT: Duration = Duration::from_secs(12);

/// 取消息最多等这么久。matrix-sdk 的通知客户端有自己的超时（缺房间密钥时最多再等几秒），这里兜底，隧道卡住时
/// 也能在系统给的后台时间里给出结果。
const FETCH_WAIT: Duration = Duration::from_secs(15);

/// 等家里的连接就绪最多这么久：确认卡片的推送要经隧道取卡片（ADR-0016），和 [`SESSION_WAIT`] 一个道理。
const HOME_WAIT: Duration = Duration::from_secs(12);

/// 推送解开后的明文，字段同核心的 `DevicePayload`（核心有契约测试锁住字段名）。确认卡片的推送（ADR-0016）
/// 不是房间里的消息：只有 `server` 和 `confirmation` 两段，凭卡片 ID 经单张接口取卡片。
#[derive(Clone, Deserialize)]
pub(crate) struct Payload {
    /// 发推送的服务器的 endpoint ID。
    pub(crate) server: String,
    #[serde(default)]
    room_id: Option<String>,
    #[serde(default)]
    event_id: Option<String>,
    sender: Option<String>,
    sender_display_name: Option<String>,
    room_name: Option<String>,
    /// 新的联系人请求（ADR-0045）：不对应房间里的消息，`room_id`、`event_id` 为空。
    #[serde(default)]
    contact_request: Option<ContactRequestPush>,
    #[serde(default)]
    confirmation: Option<DeviceConfirmationPush>,
}

impl Payload {
    /// 不是家里推送代理发的格式时返回 `InvalidInput`。
    pub(crate) fn parse(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|e| {
            invalid(&l10n!(
                CATALOG,
                language(),
                "push-payload-invalid",
                reason = e.to_string()
            ))
        })
    }

    /// 联系人请求的通知：推送里带齐了要显示的原值，不用取消息。类别同确认卡片（“等你确认”），正文的位置放
    /// `Invite`：壳看到 `contact_request` 按它写文字。
    fn contact_request(&self) -> Option<PushNotification> {
        let request = self.contact_request.clone()?;
        Some(PushNotification {
            server_id: None,
            room_id: String::new(),
            event_id: String::new(),
            sender: request
                .display_name
                .clone()
                .or_else(|| Some(request.user_id.clone())),
            room_name: None,
            content: PushContent::Invite,
            category: NotificationCategory::Confirmation,
            confirmation: None,
            contact_request: Some(request),
            thread_root: None,
        })
    }

    /// 取不到正文时的通知：只有推送里带的发送者和房间，原因照实写。
    pub(crate) fn unavailable(&self, reason: ChatFailure, message: String) -> PushNotification {
        PushNotification {
            // 槽位由 `ClientCore` 填：推送这一层不知道自己属于哪个槽位。
            server_id: None,
            room_id: self.room_id.clone().unwrap_or_default(),
            event_id: self.event_id.clone().unwrap_or_default(),
            sender: self
                .sender_display_name
                .clone()
                .or_else(|| self.sender.clone()),
            room_name: self.room_name.clone(),
            content: PushContent::Unavailable { reason, message },
            category: NotificationCategory::Message,
            confirmation: None,
            contact_request: None,
            thread_root: None,
        }
    }
}

impl PushLink {
    /// 不用显示（按推送规则不该响、消息已撤回）时返回 `None`。
    pub(crate) async fn resolve(
        &self,
        payload: &Payload,
    ) -> Result<Option<PushNotification>, CoreError> {
        if let Some(notification) = payload.contact_request() {
            return Ok(Some(notification));
        }
        if let Some(confirmation) = &payload.confirmation {
            return Ok(self.resolve_confirmation(confirmation).await);
        }
        let started = Instant::now();
        let room_id = payload
            .room_id
            .as_deref()
            .ok_or_else(|| {
                invalid(&l10n!(
                    CATALOG,
                    language(),
                    "push-payload-invalid",
                    reason = "no room_id"
                ))
            })?
            .parse::<OwnedRoomId>()
            .map_err(|e| {
                invalid(&l10n!(
                    CATALOG,
                    language(),
                    "push-invalid-room-id",
                    reason = e.to_string()
                ))
            })?;
        let event_id = payload
            .event_id
            .as_deref()
            .ok_or_else(|| {
                invalid(&l10n!(
                    CATALOG,
                    language(),
                    "push-payload-invalid",
                    reason = "no event_id"
                ))
            })?
            .parse::<OwnedEventId>()
            .map_err(|e| {
                invalid(&l10n!(
                    CATALOG,
                    language(),
                    "push-invalid-event-id",
                    reason = e.to_string()
                ))
            })?;
        let unavailable = |reason, message| payload.unavailable(reason, message);
        let session = match self.wait_session().await {
            Ok(session) => session,
            Err((reason, message)) => {
                tracing::info!(elapsed = ?started.elapsed(), %message, "push: the session isn't ready");
                return Ok(Some(unavailable(reason, message)));
            }
        };
        let session_ready = started.elapsed();
        let fetched =
            tokio::time::timeout(FETCH_WAIT, session.notification(&room_id, &event_id)).await;
        tracing::info!(?session_ready, elapsed = ?started.elapsed(), "push: finished fetching the message");
        let message = match fetched {
            Err(_) => {
                let message = l10n!(
                    CATALOG,
                    language(),
                    "push-fetch-timeout",
                    seconds = FETCH_WAIT.as_secs()
                );
                return Ok(Some(unavailable(
                    ChatFailure::TransportUnavailable,
                    message,
                )));
            }
            Ok(Err(error)) => {
                let message = l10n!(
                    CATALOG,
                    language(),
                    "push-fetch-failed",
                    reason = error.to_string()
                );
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

    /// 等聊天登录（或从本机恢复）好。登录失败、超时都给出原因；正在重连的等到超时。
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
                    return Err((
                        ChatFailure::Upstream,
                        l10n!(CATALOG, language(), "push-core-closed"),
                    ));
                }
            }
        };
        match tokio::time::timeout(SESSION_WAIT, waiting).await {
            Ok(result) => result,
            Err(_) => Err(match (self.chat.view().account, self.home.view().status) {
                (_, HomeStatus::NotConfigured) => (
                    ChatFailure::NotReady,
                    l10n!(CATALOG, language(), "push-not-paired"),
                ),
                (_, HomeStatus::Disconnected { message, .. }) => (
                    ChatFailure::TransportUnavailable,
                    l10n!(
                        CATALOG,
                        language(),
                        "push-home-unreachable",
                        reason = message
                    ),
                ),
                // 正在重连的接着等，等不到就报上一次登录失败的原因。
                (ChatAccount::Reconnecting { reason, message }, _) => (reason, message),
                (_, HomeStatus::Connecting | HomeStatus::Connected) => (
                    ChatFailure::TransportUnavailable,
                    l10n!(
                        CATALOG,
                        language(),
                        "push-session-timeout",
                        seconds = SESSION_WAIT.as_secs()
                    ),
                ),
            }),
        }
    }

    /// 确认卡片的推送（ADR-0016）：凭卡片 ID 经隧道取一张卡片。还在等回答的弹通知，点开进来源会话或
    /// “等你确认”列表；已经结束的不弹（设备从列表知道）；取不到的照实说取不到，用推送里带的标题。
    async fn resolve_confirmation(
        &self,
        confirmation: &DeviceConfirmationPush,
    ) -> Option<PushNotification> {
        let unavailable = |reason: ChatFailure| PushNotification {
            server_id: None,
            // 卡片不是房间里的消息：没有房间和事件，点开去哪儿由 `confirmation.origin` 决定。
            room_id: String::new(),
            event_id: String::new(),
            sender: None,
            room_name: None,
            content: PushContent::Unavailable {
                reason,
                message: confirmation.title.clone(),
            },
            category: NotificationCategory::Confirmation,
            confirmation: None,
            contact_request: None,
            thread_root: None,
        };
        let Some((core_url, proxy)) = self.wait_home().await else {
            tracing::info!("push: the home connection isn't ready for a confirmation");
            return Some(unavailable(ChatFailure::TransportUnavailable));
        };
        match fetch_card(core_url, proxy, &confirmation.id).await {
            // 核心说没有这张卡片：它早就结束被移出了，或者核心重启过把它忘了。不用再提醒。
            Ok(None) => None,
            Ok(Some(card)) if card.state == ConfirmationState::Pending => {
                let origin = card.origin.clone();
                Some(PushNotification {
                    server_id: None,
                    // 有来源会话的进那个房间，没有的（核心自己发起的）进“等你确认”列表。
                    room_id: origin
                        .as_ref()
                        .map(|origin| origin.room_id.clone())
                        .unwrap_or_default(),
                    event_id: card.id.clone(),
                    sender: None,
                    room_name: None,
                    content: PushContent::Message {
                        summary: MessageSummary::Text {
                            text: card.title.clone(),
                        },
                    },
                    category: NotificationCategory::Confirmation,
                    confirmation: Some(PushConfirmation {
                        id: card.id,
                        expires_at_ms: card.expires_at_ms,
                        origin,
                    }),
                    contact_request: None,
                    thread_root: None,
                })
            }
            // 已经结束的：不弹。
            Ok(Some(_)) => None,
            Err(problem) => Some(unavailable(problem.reason)),
        }
    }

    /// 等家里的连接就绪最多这么久：推送唤醒的进程要先建好隧道、拿到核心的地址。等不到就照实说取不到。
    async fn wait_home(&self) -> Option<(Url, ProxySettings)> {
        let mut home = self.home.subscribe();
        let waiting = async {
            loop {
                let proxy = home.borrow_and_update().proxy.clone();
                if let Some(proxy) = proxy
                    && let Some(core_url) = self.home.core_url()
                {
                    return Some((core_url, proxy));
                }
                if home.changed().await.is_err() {
                    return None;
                }
            }
        };
        tokio::time::timeout(HOME_WAIT, waiting)
            .await
            .unwrap_or(None)
    }
}

/// 推送和保持连接的后台通知共用的映射（ADR-0003 的“设计：保持连接”）：发送者、房间名、类别、
/// 取不到正文的占位。确认卡片的推送不在这里：它不是房间里的消息，见
/// [`PushLink::resolve_confirmation`](PushLink)。
pub(crate) fn push_notification(
    server_id: Option<String>,
    own_user: &UserId,
    room_id: &RoomId,
    event_id: &EventId,
    message: Notification,
) -> Option<PushNotification> {
    let notice = plugin_notice(own_user, &message.sender, &message.custom);
    let content = match message.body {
        MessageBody::Undecryptable => PushContent::Unavailable {
            reason: ChatFailure::NotReady,
            message: l10n!(CATALOG, language(), "push-room-key-missing"),
        },
        MessageBody::Other { kind } if kind == "m.room.member" => PushContent::Invite,
        body => PushContent::Message {
            summary: summary(body, false, &LinkTargets::default()),
        },
    };
    Some(PushNotification {
        server_id,
        room_id: room_id.to_string(),
        event_id: event_id.to_string(),
        sender: Some(message.sender_name),
        room_name: message.room_name,
        content,
        category: if notice.is_some() {
            NotificationCategory::PluginNotice
        } else {
            NotificationCategory::Message
        },
        confirmation: None,
        contact_request: None,
        thread_root: None,
    })
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

    /// 邀请和没带发送者的推送只给类别，通知上的话由壳写。
    #[test]
    fn invites_and_unknown_senders_are_left_for_the_shell_to_word() {
        let owner: OwnedUserId = "@owner:test".parse().unwrap();
        let room: wonton_matrix::OwnedRoomId = "!room:test".parse().unwrap();
        let event: OwnedEventId = "$invite".parse().unwrap();
        let invite = Notification {
            sender: "@friend:test".parse().unwrap(),
            sender_name: "Friend".into(),
            room_name: Some("Family".into()),
            body: MessageBody::Other {
                kind: "m.room.member".into(),
            },
            custom: Map::new(),
        };
        let shown = push_notification(None, &owner, &room, &event, invite).unwrap();
        assert_eq!(shown.content, PushContent::Invite);
        assert_eq!(shown.sender.as_deref(), Some("Friend"));

        let payload =
            Payload::parse(br#"{"server":"s","room_id":"!room:test","event_id":"$e"}"#).unwrap();
        let unavailable = payload.unavailable(ChatFailure::TransportUnavailable, "offline".into());
        assert_eq!(unavailable.sender, None);
    }

    /// 核心自己发的联系人请求推送不对应消息：不去取正文，直接是一张“等你确认”。
    #[test]
    fn a_contact_request_push_is_a_confirmation_without_a_message() {
        let payload = Payload::parse(
            br#"{"server":"s","room_id":"","event_id":"","contact_request":{
                "bot":{"plugin_id":"ai.nous.hermes","bot":"bob"},
                "user_id":"@hermes-alice:a.example","display_name":"Hermes",
                "keeper_display_name":"Alice"}}"#,
        )
        .unwrap();
        let shown = payload.contact_request().unwrap();
        assert_eq!(shown.category, NotificationCategory::Confirmation);
        assert_eq!(shown.sender.as_deref(), Some("Hermes"));
        let request = shown.contact_request.unwrap();
        assert_eq!(request.bot.bot, "bob");
        assert_eq!(request.keeper_display_name.as_deref(), Some("Alice"));

        let message =
            Payload::parse(br#"{"server":"s","room_id":"!room:test","event_id":"$e"}"#).unwrap();
        assert!(message.contact_request().is_none());
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

    /// 确认卡片的推送只带 `server` 和 `confirmation` 两段（ADR-0016）：不是房间里的消息，没有 room/event。
    #[test]
    fn a_confirmation_push_has_no_room_or_event() {
        let payload = Payload::parse(
            br#"{"server":"s","confirmation":{"id":"abc","title":"Hermes wants to install Grafana","expires_at_ms":1}}"#,
        )
        .unwrap();
        assert_eq!(payload.server, "s");
        let confirmation = payload.confirmation.expect("带 confirmation 段");
        assert_eq!(confirmation.id, "abc");
        assert_eq!(confirmation.title, "Hermes wants to install Grafana");
        assert_eq!(confirmation.origin, None);
        // 带来源会话的也读得懂。
        let payload = Payload::parse(
            br#"{"server":"s","confirmation":{"id":"abc","title":"t","expires_at_ms":1,"origin":{"room_id":"!r:home.example"}}}"#,
        )
        .unwrap();
        let confirmation = payload.confirmation.expect("带 confirmation 段");
        assert_eq!(
            confirmation.origin.map(|origin| origin.room_id),
            Some("!r:home.example".to_owned())
        );
        // 消息推送照旧带 room_id 和 event_id，confirmation 缺省没有。
        let payload =
            Payload::parse(br#"{"server":"s","room_id":"!room:test","event_id":"$e"}"#).unwrap();
        assert!(payload.room_id.is_some());
        assert!(payload.confirmation.is_none());
    }
}
