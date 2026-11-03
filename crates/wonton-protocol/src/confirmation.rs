//! 敏感操作的确认（ADR-0016）：核心发给主人的确认卡片，以及主人的回答。
//!
//! 卡片是核心账号（`@<CORE_LOCALPART>:<server_name>`）发的一条 `m.notice`，结构化内容放在消息内容的
//! [`CARD_FIELD`] 字段，`body` 是给其他客户端看的纯文本。状态变化时核心编辑这条消息（`m.replace`）。客户端只认
//! 核心账号发的卡片，别人发的同样结构的内容当普通消息显示。主人点了按钮，客户端经隧道请求
//! `POST <核心地址>/_wonton/confirmations/{id}`（[`answer_path`]），请求体是 [`ConfirmationAnswer`]，
//! 响应是 [`ConfirmationReply`]；回答过的、过期的得到 409。

use serde::{Deserialize, Serialize};

use crate::ChatProblem;

/// 消息内容里放卡片的字段。
pub const CARD_FIELD: &str = "app.wonton.confirmation";

/// 核心账号的用户名：只有 `@wonton-core:<server_name>` 发的卡片有按钮。
pub const CORE_LOCALPART: &str = "wonton-core";

/// 回答确认的接口路径。
pub fn answer_path(id: &str) -> String {
    format!("/_wonton/confirmations/{id}")
}

/// 一张确认卡片。文字由核心写好，客户端原样渲染。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConfirmationCard {
    /// 这次确认的 ID，回答时带上。
    pub id: String,
    pub state: ConfirmationState,
    /// 一句话说明谁要做什么，如“Hermes 想安装 Grafana”。
    pub title: String,
    /// 说明行，按顺序显示。
    pub lines: Vec<CardLine>,
    /// 按钮，按顺序显示；只在 [`ConfirmationState::Pending`] 时可点。
    pub choices: Vec<CardChoice>,
    /// 到期时间，Unix 毫秒。
    pub expires_at_ms: u64,
}

/// 卡片上的一行说明。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CardLine {
    pub text: String,
    /// 醒目显示：高权限、删除数据这类主人必须看清的内容。
    pub emphasis: bool,
}

/// 卡片上的一个按钮。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CardChoice {
    pub answer: Answer,
    pub label: String,
}

/// 主人的回答。
///
/// 核心和 App 分开升级（ADR-0012）：核心可能给出这个 App 不认识的按钮，App 不显示它，卡片上别的按钮照常能点；核心
/// 收到不认识的回答返回 400，绝不当成同意。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    Approve,
    Reject,
    /// 不认识的回答，不能发给核心。
    #[serde(other)]
    Unknown,
}

/// 确认的状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConfirmationState {
    /// 等主人回答。
    Pending,
    /// 主人确认了，正在执行。
    Running,
    /// 执行完了；`message` 是结果，比如装好的插件和它的网页地址。
    Succeeded { message: String },
    /// 执行失败；`message` 是原因。
    Failed { message: String },
    /// 发给插件了，但没等到结果（超时、连接断了）：可能已经执行了。`message` 写明为什么不确定。
    Uncertain { message: String },
    /// 主人拒绝了，没有执行。
    Rejected,
    /// 主人没在期限内回答，没有执行。
    Expired,
}

/// `POST /_wonton/confirmations/{id}` 的请求体。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConfirmationAnswer {
    pub answer: Answer,
}

/// 回答之后卡片的状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConfirmationReply {
    pub state: ConfirmationState,
}

/// 时间线里的一张确认卡片，发件人已经核对过是核心账号（ADR-0016）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConfirmationView {
    /// 客户端核心已经算上了自己知道的：期限过了没等到核心的更新，`state` 就是过期；核心收下回答后，`state` 不等卡片
    /// 更新就变。`choices` 只有现在能回答的按钮，不等回答时为空。
    pub card: ConfirmationCard,
    /// 正在发给核心的回答。这时按钮还在，但不能点。
    pub sending: Option<Answer>,
    /// 最近一次回答没成的原因。连接断了、核心出错时按钮还在，可以再点；核心不收（回答过了、没有这个确认）时按钮
    /// 消失，原因是 `InvalidInput`。
    pub problem: Option<ChatProblem>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn answers_and_states_have_a_stable_wire_format() {
        let answer: ConfirmationAnswer =
            serde_json::from_value(json!({"answer": "approve"})).unwrap();
        assert_eq!(answer.answer, Answer::Approve);
        let state = ConfirmationState::Failed {
            message: "x".into(),
        };
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            json!({"type": "failed", "message": "x"})
        );
        let state = ConfirmationState::Uncertain {
            message: "x".into(),
        };
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            json!({"type": "uncertain", "message": "x"})
        );
        assert_eq!(answer_path("abc"), "/_wonton/confirmations/abc");
    }

    /// 契约：核心比 App 新时，卡片多了不认识的按钮和字段也能解析，那个按钮落到 [`Answer::Unknown`]。
    #[test]
    fn a_newer_core_does_not_break_the_card() {
        let card: ConfirmationCard = serde_json::from_value(json!({
            "id": "abc",
            "state": {"type": "pending"},
            "title": "Hermes 想用 Grafana 的 query",
            "lines": [],
            "choices": [
                {"answer": "approve", "label": "允许"},
                {"answer": "allow_window", "label": "30 分钟内都允许"},
                {"answer": "reject", "label": "拒绝"},
            ],
            "expires_at_ms": 1,
            "window": {"expires_at_ms": 2},
        }))
        .unwrap();
        let answers: Vec<_> = card.choices.iter().map(|choice| choice.answer).collect();
        assert_eq!(answers, [Answer::Approve, Answer::Unknown, Answer::Reject]);
    }
}
