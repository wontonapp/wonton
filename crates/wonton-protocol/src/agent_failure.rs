//! Agent 一轮失败时在房间里发的结构化说明。
//!
//! 失败消息仍是普通的 `m.text`：[`crate::timeline::TimelineMessage`] 的 `body` 是给所有 Matrix
//! 客户端看的人话（按说话人的语言），类别和原始报错放在消息内容的平级字段
//! [`AGENT_FAILURE_FIELD`] 里。App 把它画成失败卡：按类别的一句人话、能做的动作，原始报错收进
//! 默认收起的“详情”，按原文显示，不识别链接。客户端只认 Bot 账号发的，别人发的同样结构的
//! 内容当普通消息；不认识的类别落 [`AgentFailureCategory::Other`]。

use serde::{Deserialize, Serialize};

/// 消息内容里放一轮失败说明的字段。
pub const AGENT_FAILURE_FIELD: &str = "app.wonton.agent_failure";

/// 一轮失败的说明，消息内容里 [`AGENT_FAILURE_FIELD`] 字段的值。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgentFailure {
    pub category: AgentFailureCategory,
    /// 原始报错，给默认收起的“详情”用；发送方截到 2000 字符。
    pub detail: String,
}

/// 一轮失败的原因类别。类别定用户能做的事，人话由壳按类别写，不在这里带文案。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AgentFailureCategory {
    /// 请求没送到模型服务：连不上、域名解析不了、TLS 失败、网关 502/504。
    Unreachable,
    /// 模型服务在，但拒绝了凭据：401/403，key 无效或被吊销。要去模型设置换 key。
    KeyRejected,
    /// 凭据有效但额度用完或被限流：402、429。等额度恢复或换模型。
    QuotaExhausted,
    /// 请求到了，上游返回错误：500/503/529、无效响应、内容策略拒绝。
    UpstreamError,
    /// 这一轮没走完就停了：被打断、流超时、会话重启。部分动作可能已经执行过。
    TurnInterrupted,
    /// 不认识的类别：插件比 App 新。
    #[serde(other)]
    Other,
}

/// 失败卡上可以给主人的动作。哪些动作用得上由核心按类别决定，壳照列表画按钮。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AgentFailureAction {
    /// 重发主人上一条消息。
    Retry,
    /// 打开模型设置。
    OpenModelSettings,
}

/// 时间线里给壳的失败卡：类别、原文和动作都由核心给出，壳只渲染。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgentFailureCard {
    pub category: AgentFailureCategory,
    /// 原始报错，壳原样显示，不排版、不识别链接。
    pub detail: String,
    /// 能做的动作，按显示顺序。
    pub actions: Vec<AgentFailureAction>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// 契约：线上的字段形状读得懂，不认识的类别落 `Other`，多出来的键忽略。
    #[test]
    fn the_wire_shape_reads_and_tolerates_new_values() {
        let failure: AgentFailure = serde_json::from_value(json!({
            "category": "unreachable",
            "detail": "Provider said: Post \"http://192.0.2.10:8080/v1/chat/completions\": 502 Bad Gateway",
        }))
        .unwrap();
        assert_eq!(failure.category, AgentFailureCategory::Unreachable);
        let stale: AgentFailure = serde_json::from_value(json!({
            "category": "uplink_rebooted",
            "detail": "…",
            "retry_after_ms": 30_000,
        }))
        .unwrap();
        assert_eq!(stale.category, AgentFailureCategory::Other);
        for (category, wire) in [
            (AgentFailureCategory::Unreachable, "unreachable"),
            (AgentFailureCategory::KeyRejected, "key_rejected"),
            (AgentFailureCategory::QuotaExhausted, "quota_exhausted"),
            (AgentFailureCategory::UpstreamError, "upstream_error"),
            (AgentFailureCategory::TurnInterrupted, "turn_interrupted"),
        ] {
            assert_eq!(serde_json::to_value(category).unwrap(), json!(wire));
        }
    }

    /// 契约：动作和失败卡都能经 JSON 往返。
    #[test]
    fn actions_and_cards_round_trip_through_json() {
        for action in [AgentFailureAction::Retry, AgentFailureAction::OpenModelSettings] {
            assert_eq!(
                serde_json::from_value::<AgentFailureAction>(serde_json::to_value(action).unwrap())
                    .unwrap(),
                action
            );
        }
        let card = AgentFailureCard {
            category: AgentFailureCategory::KeyRejected,
            detail: "Provider said: 401 Unauthorized".into(),
            actions: vec![AgentFailureAction::Retry, AgentFailureAction::OpenModelSettings],
        };
        assert_eq!(
            serde_json::from_str::<AgentFailureCard>(&serde_json::to_string(&card).unwrap())
                .unwrap(),
            card
        );
    }
}
