//! 敏感操作的确认（ADR-0016）：核心发给主人的确认卡片，以及主人的回答。
//!
//! 卡片是核心账号（`@<CORE_LOCALPART>:<server_name>`）发的一条 `m.text`，结构化内容放在消息内容的
//! [`CARD_FIELD`] 字段，`body` 是给其他客户端看的纯文本。状态变化时核心编辑这条消息（`m.replace`）。客户端只认
//! 核心账号发的卡片，别人发的同样结构的内容当普通消息显示。主人点了按钮，客户端经隧道请求
//! `POST <核心地址>/_wonton/confirmations/{id}`（[`answer_path`]），请求体是 [`ConfirmationAnswer`]，
//! 响应是 [`ConfirmationReply`]；回答过的、过期的得到 409。没有配对设备的地方（开发环境）用主人令牌请求同一个接口，
//! 等回答的卡片从 `GET /_wonton/api/confirmations`（[`ConfirmationList`]）取。

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

/// 停止放行时段的接口路径（`DELETE`，ADR-0024）。
pub fn window_path(id: &str) -> String {
    format!("/_wonton/confirmations/{id}/window")
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
    /// 这张卡片打开的放行时段（ADR-0024）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<ConfirmationWindow>,
    /// 这张卡片要装或要更新的插件（ADR-0033）。批准后插件有网页时，壳显示“打开”和“放到底栏”。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// 装插件、升级插件的卡片要主人填的密钥（ADR-0064）：壳按它渲染原生表单，值随回答的
    /// [`ConfirmationAnswer::secrets`] 带回。别的卡片是空列表。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_inputs: Vec<crate::SecretInput>,
}

/// 放行时段：这个 Bot 调这个插件，到期前不再发卡片。卡片只在时段开始和结束时更新。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConfirmationWindow {
    /// 到期时间，Unix 毫秒。
    pub expires_at_ms: u64,
    /// 放行了多少次，不算打开时段的这次。核心重启后说不清，是 `None`。
    pub allowed: Option<u32>,
    /// 结束的原因。还没结束时是 `None`，这时显示“停止”按钮。
    pub ended: Option<WindowEnd>,
}

/// 放行时段为什么结束。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum WindowEnd {
    Expired,
    Stopped,
    CoreRestarted,
    /// 不认识的原因：核心比 App 新。
    #[serde(other)]
    Unknown,
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
    /// Agent 自己的选项（ADR-0035）：一张卡片上可以有多个同为同意或拒绝的按钮，靠它区分。回答时原样带回。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub choice_id: Option<String>,
}

/// 主人的回答。不认识的值怎么处理见 ADR-0016 第 3 步。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    Approve,
    /// 同意，范围比 [`Answer::Approve`] 大一层：存文件的卡片上是“允许存进整个共享文件夹”（ADR-0027）。
    ApproveWider,
    /// 同意，并在一段时间内放行这个 Bot 对这个插件的调用（ADR-0024）。时长由核心决定。
    AllowWindow,
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

/// `GET /_wonton/api/confirmations` 的响应：等回答的卡片，最早到期的在前。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmationList {
    pub confirmations: Vec<ConfirmationCard>,
}

/// `POST /_wonton/confirmations/{id}` 的请求体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConfirmationAnswer {
    pub answer: Answer,
    /// 点的那个按钮的 [`CardChoice::choice_id`]。和卡片上的按钮对不上时返回 400 `invalid_input`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub choice_id: Option<String>,
    /// 装插件、升级插件的卡片要主人填的密钥（ADR-0064），键是卡片列的项；别的卡片带上时返回 400 `invalid_input`。
    #[serde(default)]
    pub secrets: crate::SecretValues,
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
    /// 更新就变。`choices` 只有现在能回答的按钮，不等回答时为空，[`Answer::Unknown`] 的按钮也已去掉，壳直接渲染。
    pub card: ConfirmationCard,
    /// 正在发给核心的回答。这时按钮还在，但不能点。
    pub sending: Option<Answer>,
    /// 正在发的是哪个按钮：[`Self::sending`] 相同的按钮靠它区分。
    pub sending_choice_id: Option<String>,
    /// 最近一次回答没成的原因。连接断了、核心出错时按钮还在，可以再点；核心不收（回答过了、没有这个确认）时按钮
    /// 消失，原因是 `InvalidInput`。
    pub problem: Option<ChatProblem>,
    /// 这张卡片打开的放行时段现在的样子，已经算上这台设备知道的：到期了、自己点了“停止”。
    pub window: Option<WindowView>,
}

/// 卡片上的放行时段（ADR-0024）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum WindowView {
    /// 生效中：显示到期时间和“停止”。
    Active {
        expires_at_ms: u64,
        /// 正在发“停止”。这时按钮还在，但不能点。
        stopping: bool,
        /// 最近一次“停止”没成的原因。时段还在生效，可以再点。
        problem: Option<ChatProblem>,
    },
    Ended {
        note: String,
    },
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
                {"answer": "ask_later", "label": "稍后再问"},
                {"answer": "reject", "label": "拒绝"},
            ],
            "expires_at_ms": 1,
            "window": {"expires_at_ms": 2, "allowed": 1, "ended": "new_reason"},
            "plugin_id": "app.rss",
            "extra": {"expires_at_ms": 2},
        }))
        .unwrap();
        let answers: Vec<_> = card.choices.iter().map(|choice| choice.answer).collect();
        assert_eq!(answers, [Answer::Approve, Answer::Unknown, Answer::Reject]);
        assert_eq!(card.window.unwrap().ended, Some(WindowEnd::Unknown));
        assert_eq!(card.plugin_id.as_deref(), Some("app.rss"));
    }

    /// 契约：`choice_id` 是可选的，没有时线上不出现，`Answer` 的方向不变。
    #[test]
    fn a_choice_is_told_apart_by_its_id() {
        let plain = CardChoice {
            answer: Answer::Approve,
            label: "允许".into(),
            choice_id: None,
        };
        assert_eq!(json!(plain), json!({"answer": "approve", "label": "允许"}));
        let always: CardChoice = serde_json::from_value(json!({
            "answer": "approve",
            "label": "总是允许",
            "choice_id": "opaque-2",
        }))
        .unwrap();
        assert_eq!(always.answer, Answer::Approve);
        assert_eq!(always.choice_id.as_deref(), Some("opaque-2"));

        let answer = ConfirmationAnswer {
            answer: Answer::Approve,
            choice_id: Some("opaque-2".into()),
            secrets: crate::SecretValues::new(),
        };
        assert_eq!(
            json!(answer),
            json!({"answer": "approve", "choice_id": "opaque-2", "secrets": {}})
        );
        let answer: ConfirmationAnswer =
            serde_json::from_value(json!({"answer": "reject"})).unwrap();
        assert_eq!(answer.choice_id, None);
        assert!(answer.secrets.is_empty(), "旧 App 不带密钥表，按空算");

        let answer: ConfirmationAnswer = serde_json::from_value(json!({
            "answer": "approve",
            "secrets": {"brave_key": "s3cret"},
        }))
        .unwrap();
        assert_eq!(answer.secrets["brave_key"].expose(), "s3cret");
        assert_eq!(
            format!("{:?}", answer.secrets["brave_key"]),
            "<redacted>",
            "回答里填的值不泄露"
        );
    }

    /// 契约：卡片的 JSON 形状。
    #[test]
    fn cards_round_trip_through_json() {
        let card: ConfirmationCard = serde_json::from_value(json!({
            "id": "abc",
            "state": {"type": "pending"},
            "title": "Alex 的服务器想和你的服务器连接",
            "lines": [],
            "choices": [
                {"answer": "approve", "label": "同意"},
                {"answer": "reject", "label": "拒绝"},
            ],
            "expires_at_ms": 1,
        }))
        .unwrap();
        assert_eq!(card.title, "Alex 的服务器想和你的服务器连接");
    }

    #[test]
    fn the_wider_approval_has_a_wire_name() {
        assert_eq!(json!(Answer::ApproveWider), json!("approve_wider"));
    }

    #[test]
    fn a_window_has_a_stable_wire_format() {
        assert_eq!(json!(Answer::AllowWindow), json!("allow_window"));
        let window = ConfirmationWindow {
            expires_at_ms: 2,
            allowed: None,
            ended: Some(WindowEnd::CoreRestarted),
        };
        assert_eq!(
            json!(window),
            json!({"expires_at_ms": 2, "allowed": null, "ended": "core_restarted"})
        );
        assert_eq!(window_path("abc"), "/_wonton/confirmations/abc/window");
    }
}
