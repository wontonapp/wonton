//! 出箱（ADR-0061 第 9 节）：核心把批准的消息和要取消的轮次放进 `GET` [`AGENT_OUTBOX_PATH`]，
//! 前端（桥）长轮询取走，做完按 [`OutboxReceipt`] 回报。路径和 `/agent/permissions` 同一道门，
//! 只有总线令牌的请求被拒。
//!
//! 这里的项都由核心产出、前端消费：不认识的项落到 [`OutboxKind::Other`]，前端照实回报并跳过，
//! 不猜怎么执行。

use serde::{Deserialize, Serialize};

/// 取走要执行的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxItem {
    /// 核心发的项 ID；回报时原样带回，重发同一个私聊消息也拿它当幂等键。
    pub id: String,
    pub kind: OutboxKind,
}

/// 出箱项要做什么。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutboxKind {
    /// 把正文以 Bot 的身份发进和收件人的私聊（ADR-0061 第 9 节）：主人已经批准了
    /// [`crate::ConfirmationCategory::SendMessage`] 那张卡片。
    Message { recipient: String, body: String },
    /// 这条链的这个会话的一轮被停链取消了：前端对它发 `session/cancel`（ADR-0061 第 10 节），
    /// 同会话别的待答请求一并回 `cancelled`，然后回报结果。
    CancelTurn { chain: String, session: String },
    /// 比前端新的核心给的未知项。
    #[serde(other)]
    Other,
}

/// 前端对取走的一项的回报，路径同 [`AGENT_OUTBOX_PATH`]。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxReceipt {
    /// 回报的是哪个 [`OutboxItem::id`]。
    pub id: String,
    pub result: OutboxResult,
}

/// 一项做完了没有。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum OutboxResult {
    /// 已发出；`event_id` 是私聊里那条消息的事件 ID（核心记进审计）。`CancelTurn` 发成后也报
    /// 这一项，`event_id` 为空字符串——取消不是房间里的事件。
    Sent { event_id: String },
    /// 没做成；为什么见 [`OutboxFailure`]。
    Failed { category: OutboxFailure },
}

/// 出箱项为什么没做成。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxFailure {
    /// 没有和收件人的私聊；建私聊要 ADR-0083 的房间表，现在先这样报。
    NoRoom,
    /// 比核心方新的前端给的未知原因。
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_message_item_round_trips() {
        let item = OutboxItem {
            id: "o-1".into(),
            kind: OutboxKind::Message {
                recipient: "mom".into(),
                body: "周末回来吗".into(),
            },
        };
        assert_eq!(
            json!(item),
            json!({
                "id": "o-1",
                "kind": {"type": "message", "recipient": "mom", "body": "周末回来吗"},
            })
        );
        assert_eq!(
            serde_json::from_value::<OutboxItem>(json!(item)).unwrap(),
            item
        );
    }

    #[test]
    fn a_cancel_turn_item_and_a_receipt_round_trip() {
        let item = OutboxItem {
            id: "o-2".into(),
            kind: OutboxKind::CancelTurn {
                chain: "c-1".into(),
                session: "s-1".into(),
            },
        };
        assert_eq!(
            json!(item),
            json!({
                "id": "o-2",
                "kind": {"type": "cancel_turn", "chain": "c-1", "session": "s-1"},
            })
        );
        assert_eq!(
            serde_json::from_value::<OutboxItem>(json!(item)).unwrap(),
            item
        );

        let receipt = OutboxReceipt {
            id: "o-2".into(),
            result: OutboxResult::Failed {
                category: OutboxFailure::NoRoom,
            },
        };
        assert_eq!(
            json!(receipt),
            json!({"id": "o-2", "result": {"result": "failed", "category": "no_room"}})
        );
        assert_eq!(
            serde_json::from_value::<OutboxReceipt>(json!(receipt)).unwrap(),
            receipt
        );
    }

    /// 契约：不认识的项和原因落到 `Other`，核心比前端新时前端不猜怎么执行。
    #[test]
    fn unknown_kinds_fall_back_to_other() {
        assert_eq!(
            serde_json::from_value::<OutboxKind>(json!({"type": "ping"})).unwrap(),
            OutboxKind::Other
        );
        let item = serde_json::from_value::<OutboxItem>(json!({
            "id": "o-3",
            "kind": {"type": "invite", "who": "@new:home.example"},
        }))
        .unwrap();
        assert_eq!(item.id, "o-3");
        assert_eq!(item.kind, OutboxKind::Other);
        assert_eq!(
            serde_json::from_value::<OutboxResult>(json!({
                "result": "failed", "category": "rate_limited",
            }))
            .unwrap(),
            OutboxResult::Failed {
                category: OutboxFailure::Other,
            }
        );
    }
}
