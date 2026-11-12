//! 交给另一个 Agent、查交出去的会话、替人发消息（ADR-0061 第 9 节）：Wonton MCP 总线上的三个核心工具。
//!
//! 调用方是链上的 Bot（或网页前端）；工具开好会话就立即返回，不等目标做完，目标在对话里自己说话。
//! 自调、绕圈、层数都走调用链的检查（[`crate::ChainError`]），确认卡片见 [`crate::ConfirmationCategory`]。

use serde::{Deserialize, Serialize};

use crate::BotRef;

/// 交给另一个 Agent 的核心工具（ADR-0061 第 9 节）。
pub const HAND_OFF_TOOL: &str = "wonton__hand_off";

/// 查交出去的会话的核心工具。只读：不改任何东西、不数轮数、不给目标发任何帧。
pub const HAND_OFF_STATUS_TOOL: &str = "wonton__hand_off_status";

/// 给某人发消息的核心工具（ADR-0061 第 9 节）；投递走出箱（[`crate::AGENT_OUTBOX_PATH`]）。
pub const SEND_MESSAGE_TOOL: &str = "wonton__send_message";

/// [`HAND_OFF_TOOL`] 的入参。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandOffRequest {
    /// 交活给哪个 Bot。目标 Bot 必须听链代表的人，否则 [`crate::ChainError::PermissionDenied`]。
    pub target_bot: BotRef,
    /// 交代的话，原样成为目标会话的第一轮。
    pub message: String,
}

/// [`HAND_OFF_TOOL`] 的结果：会话开好立即返回，新会话沿用调用方的链（ADR-0061 第 10 节）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandOffResult {
    /// 目标会话的 ID。同一个调用方会话里同一请求 ID 的重试、或同一目标同一句话且前一个还在做，
    /// 都返回已开的那个会话，不开新的（ADR-0061 第 9 节）。
    pub session: String,
    /// 会话所在的链 ID。
    pub chain: String,
}

/// [`HAND_OFF_STATUS_TOOL`] 的入参。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandOffStatusRequest {
    /// 查哪个会话；不填时列出调用的这个 Bot 从同一对话（房间、话题）交出去的全部。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

/// [`HAND_OFF_STATUS_TOOL`] 返回的一个交出去的会话。查别的 Bot 交出去的返回
/// [`crate::ChainError::PermissionDenied`]。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandOffSession {
    pub session: String,
    /// 目标 Bot。
    pub bot: BotRef,
    /// 会话所在的链 ID。
    pub chain: String,
    pub status: HandOffState,
    /// ACP 的结束原因（如 `end_turn`）；还没结束时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    /// 会话开好的时间，Unix 毫秒。
    pub started_at: u64,
    /// 最近一次有动静的时间，Unix 毫秒。
    pub last_activity_at: u64,
    /// 目标最近一轮回复的末尾，超长截到 2000 字。
    pub summary: String,
}

/// 交出去的会话的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum HandOffState {
    /// 还在做。
    Running,
    /// 目标在等主人确认权限；卡片带着这条链的会话（[`crate::AgentPermissionRequest::session`]）。
    WaitingPermission,
    /// 做完了。
    Ended,
    /// 链被人停了，会话跟着取消。
    Stopped,
    /// 链到了上限，会话被拒。
    LimitReached,
    /// 会话出错了。
    Failed,
    /// 比调用方新的核心给的未知状态。
    #[serde(other)]
    Other,
}

/// [`SEND_MESSAGE_TOOL`] 的入参。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendMessageRequest {
    /// 收件人：这个 Bot 归属人的家人或好友（ADR-0045、ADR-0051），按用户名认。每条都先发卡片给
    /// Bot 的归属人，卡片里是收件人和全文（[`crate::ConfirmationCategory::SendMessage`]）。
    pub recipient: String,
    /// 要发的正文，批准后原样发进和收件人的私聊。
    pub body: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_hand_off_request_and_result_round_trip() {
        let request = HandOffRequest {
            target_bot: BotRef {
                plugin_id: "app.codex".into(),
                bot: "owner".into(),
            },
            message: "修登录页".into(),
        };
        assert_eq!(
            json!(request),
            json!({
                "target_bot": {"plugin_id": "app.codex", "bot": "owner"},
                "message": "修登录页",
            })
        );
        assert_eq!(
            serde_json::from_value::<HandOffRequest>(json!(request)).unwrap(),
            request
        );

        let result = HandOffResult {
            session: "s-2".into(),
            chain: "c-1".into(),
        };
        assert_eq!(json!(result), json!({"session": "s-2", "chain": "c-1"}));
        assert_eq!(
            serde_json::from_value::<HandOffResult>(json!(result)).unwrap(),
            result
        );
    }

    #[test]
    fn a_status_request_without_a_session_lists_the_conversation() {
        let bare = HandOffStatusRequest { session: None };
        assert_eq!(json!(bare), json!({}));
        assert_eq!(
            serde_json::from_value::<HandOffStatusRequest>(json!({})).unwrap(),
            bare
        );
    }

    /// 契约：状态原样给出，结束时带结束原因；不认识的状态落到 `Other`。
    #[test]
    fn a_session_round_trips_and_unknown_states_fall_back() {
        let session = HandOffSession {
            session: "s-2".into(),
            bot: BotRef {
                plugin_id: "app.codex".into(),
                bot: "owner".into(),
            },
            chain: "c-1".into(),
            status: HandOffState::Ended,
            stop_reason: Some("end_turn".into()),
            started_at: 1_000,
            last_activity_at: 2_000,
            summary: "修好了".into(),
        };
        assert_eq!(
            json!(session),
            json!({
                "session": "s-2",
                "bot": {"plugin_id": "app.codex", "bot": "owner"},
                "chain": "c-1",
                "status": "ended",
                "stop_reason": "end_turn",
                "started_at": 1000,
                "last_activity_at": 2000,
                "summary": "修好了",
            })
        );
        assert_eq!(
            serde_json::from_value::<HandOffSession>(json!(session)).unwrap(),
            session
        );

        let mut waiting = session.clone();
        waiting.status = HandOffState::WaitingPermission;
        waiting.stop_reason = None;
        assert!(json!(waiting).get("stop_reason").is_none());
        assert_eq!(
            serde_json::from_value::<HandOffState>(json!("running")).unwrap(),
            HandOffState::Running
        );
        assert_eq!(
            serde_json::from_value::<HandOffState>(json!("paused")).unwrap(),
            HandOffState::Other
        );
    }

    #[test]
    fn a_send_message_request_round_trips() {
        let request = SendMessageRequest {
            recipient: "mom".into(),
            body: "周末回来吗".into(),
        };
        assert_eq!(
            json!(request),
            json!({"recipient": "mom", "body": "周末回来吗"})
        );
        assert_eq!(
            serde_json::from_value::<SendMessageRequest>(json!(request)).unwrap(),
            request
        );
    }
}
