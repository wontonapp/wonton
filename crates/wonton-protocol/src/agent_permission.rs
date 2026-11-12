//! Agent 权限确认（ADR-0035）：Agent 自己的工具要动手之前，经插件网络请主人确认。
//!
//! 插件网络里 `POST` MCP 总线同一个地址的 [`AGENT_PERMISSION_PATH`]，请求体是 [`AgentPermissionRequest`]。这是一个长请求：
//! 等主人回答，最长 [`AGENT_PERMISSION_TTL_SECS`] 秒，响应是 [`AgentPermissionOutcome`]。调用方按来源地址认出，和 MCP 总线
//! 一样。请求不合法（选项不认识、没有选项、`choice_id` 重复）时返回 400 `invalid_input`；已经有太多请求在等主人时返回
//! 429 `too_many_pending`；卡片发不出去时返回 503 `confirmation_unavailable`；调用方提前断开时卡片随之作废。

use serde::{Deserialize, Serialize};

use crate::Answer;

/// 请求主人确认的接口路径。
pub const AGENT_PERMISSION_PATH: &str = "/agent/permissions";

/// 卡片的期限，也是长请求最长等多久。
pub const AGENT_PERMISSION_TTL_SECS: u64 = 600;

/// `POST` [`AGENT_PERMISSION_PATH`] 的请求体。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentPermissionRequest {
    pub tool_call_id: String,
    /// 一句话说明要做什么，显示在卡片上。
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 工具的参数。调用方先去掉秘密：这里的内容会进卡片和审计。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
    /// 按顺序显示成卡片上的按钮。没有拒绝类的选项时，核心加一个“取消”按钮。
    pub options: Vec<AgentPermissionOption>,
}

/// 主人可选的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPermissionOption {
    /// 调用方自己定的不透明标识，这次请求内不重复。
    pub choice_id: String,
    pub kind: AgentPermissionKind,
    /// 按钮上的文字。
    pub name: String,
}

/// 选项的类别。这个接口的调用方和核心一起发布，不认识的类别是调用方的错，不落到 `Other`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

impl AgentPermissionKind {
    /// 卡片上这个按钮的方向。`AllowAlways` 是 Agent 自己的作用域，不等于 [`Answer::ApproveWider`]，也不写工具策略。
    pub fn answer(self) -> Answer {
        match self {
            Self::AllowOnce | Self::AllowAlways => Answer::Approve,
            Self::RejectOnce | Self::RejectAlways => Answer::Reject,
        }
    }
}

/// `POST` [`AGENT_PERMISSION_PATH`] 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AgentPermissionOutcome {
    /// 主人选了这一项，包括拒绝类的选项。
    Selected { choice_id: String },
    /// 主人点了核心加的“取消”。
    Cancelled,
    /// 主人没在期限内回答。
    Expired,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_request_has_a_stable_wire_format() {
        let request: AgentPermissionRequest = serde_json::from_value(json!({
            "tool_call_id": "call-1",
            "title": "运行 ls",
            "description": "列出工作区的文件",
            "arguments": {"command": "ls"},
            "options": [
                {"choice_id": "a", "kind": "allow_once", "name": "这次允许"},
                {"choice_id": "b", "kind": "allow_always", "name": "总是允许"},
                {"choice_id": "c", "kind": "reject_once", "name": "拒绝"},
                {"choice_id": "d", "kind": "reject_always", "name": "总是拒绝"},
            ],
        }))
        .unwrap();
        assert_eq!(request.tool_call_id, "call-1");
        assert_eq!(request.arguments, Some(json!({"command": "ls"})));
        let directions: Vec<_> = request.options.iter().map(|o| o.kind.answer()).collect();
        assert_eq!(
            directions,
            [
                Answer::Approve,
                Answer::Approve,
                Answer::Reject,
                Answer::Reject
            ],
            "两种同意都不是 ApproveWider"
        );

        let bare: AgentPermissionRequest = serde_json::from_value(json!({
            "tool_call_id": "call-2",
            "title": "运行 ls",
            "options": [],
        }))
        .unwrap();
        assert_eq!(bare.description, None);
        assert_eq!(bare.arguments, None);
    }

    /// 契约：不认识的类别解析不了，核心据此返回 `invalid_input`，不会把它当成同意或拒绝。
    #[test]
    fn an_unknown_kind_does_not_parse() {
        let option = serde_json::from_value::<AgentPermissionOption>(json!({
            "choice_id": "a",
            "kind": "allow_forever",
            "name": "永远允许",
        }));
        assert!(option.is_err());
    }

    #[test]
    fn outcomes_have_a_stable_wire_format() {
        let selected = AgentPermissionOutcome::Selected {
            choice_id: "a".into(),
        };
        assert_eq!(
            json!(selected),
            json!({"outcome": "selected", "choice_id": "a"})
        );
        assert_eq!(
            json!(AgentPermissionOutcome::Cancelled),
            json!({"outcome": "cancelled"})
        );
        assert_eq!(
            json!(AgentPermissionOutcome::Expired),
            json!({"outcome": "expired"})
        );
    }
}
