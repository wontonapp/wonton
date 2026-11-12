//! Agent 一轮任务的进度（MSC4564，ADR-0082）：Bot 在聊天里发的一条状态消息，客户端核心按最新编辑解读，
//! 变成时间线里的 [`TimelineContent::AgentRun`](crate::TimelineContent::AgentRun)。
//!
//! 状态对象是发送者的自述：客户端只对核心认得的 Bot 账号解读，从不自动作答。

use serde::{Deserialize, Serialize};

/// 一轮任务的状态。`Working`、`Waiting`、`Done`、`Failed` 是 MSC4564 的四个值；`Uncertain` 由客户端核心
/// 合成：Bot 离线或活动未知时，还停在 `Working`、`Waiting` 的状态行按它显示，不转圈（ADR-0082 第 4 节）。
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AgentRunState {
    /// 在干活。
    Working,
    /// 在等主人：等核心的确认卡片，或等 Bot 自己发的 `choices`。
    Waiting,
    /// 完成。
    Done,
    /// 失败或被打断，为什么看 `end` 和 `result`。
    Failed,
    /// 结果不确定，核心合成。
    Uncertain,
    /// MSC 以后新加的状态值：壳按普通 Bot 提示消息显示 `body`。
    Other(String),
}

impl AgentRunState {
    fn known(raw: &str) -> Option<Self> {
        match raw {
            "working" => Some(Self::Working),
            "waiting" => Some(Self::Waiting),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            "uncertain" => Some(Self::Uncertain),
            _ => None,
        }
    }
}

impl Serialize for AgentRunState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(match self {
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Uncertain => "uncertain",
            Self::Other(raw) => raw,
        })
    }
}

impl<'de> Deserialize<'de> for AgentRunState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::known(&raw).unwrap_or(Self::Other(raw)))
    }
}

/// 一轮怎么结束的（`app.wonton.end`，我们加的键）：MSC4564 没有这两种结束，别的客户端看到的是 `Failed`
/// 和 `body` 里写明的原因（ADR-0082 第 1 节）。
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AgentRunEnd {
    /// 主人打断了这一轮，壳写“打断了”，不画成失败的红色。
    Cancelled,
    /// 结果不确定，壳写“结果不确定”，不画成失败的红色。
    Uncertain,
    /// 以后核心新加的取值，壳当 `Failed` 画。
    Other(String),
}

impl AgentRunEnd {
    fn known(raw: &str) -> Option<Self> {
        match raw {
            "cancelled" => Some(Self::Cancelled),
            "uncertain" => Some(Self::Uncertain),
            _ => None,
        }
    }
}

impl Serialize for AgentRunEnd {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(match self {
            Self::Cancelled => "cancelled",
            Self::Uncertain => "uncertain",
            Self::Other(raw) => raw,
        })
    }
}

impl<'de> Deserialize<'de> for AgentRunEnd {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::known(&raw).unwrap_or(Self::Other(raw)))
    }
}

/// Bot 自己发的待回答选项（MSC4564 的 `choices`）：主人点了才经
/// [`Action::AnswerAgentChoice`](crate::Action::AnswerAgentChoice) 以主人的身份发 `m.annotation` 反应，
/// 不自动作答。它答的是 Agent 自己的问题，不经核心，也不是 Wonton 的确认。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgentRunChoice {
    /// 反应用的内容（`m.annotation` 的 `key`），MSC4564 用它区分选项。
    pub key: String,
    /// 按钮文字，Bot 给的原话，原样显示，不翻译。
    pub label: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 契约：状态的每个取值都能经 JSON 往返；线上是新加的取值时落到 `Other`，原样带回来。
    #[test]
    fn states_round_trip_and_unknown_ones_carry_their_raw_value() {
        for (state, json) in [
            (AgentRunState::Working, "working"),
            (AgentRunState::Waiting, "waiting"),
            (AgentRunState::Done, "done"),
            (AgentRunState::Failed, "failed"),
            (AgentRunState::Uncertain, "uncertain"),
        ] {
            assert_eq!(
                serde_json::to_value(&state).unwrap(),
                serde_json::json!(json)
            );
            assert_eq!(
                serde_json::from_value::<AgentRunState>(serde_json::json!(json)).unwrap(),
                state
            );
        }
        let other = AgentRunState::Other("brewing".into());
        assert_eq!(
            serde_json::from_value::<AgentRunState>(serde_json::json!("brewing")).unwrap(),
            other
        );
        assert_eq!(
            serde_json::to_value(&other).unwrap(),
            serde_json::json!("brewing")
        );
    }

    /// 契约：结束原因的每个取值都能经 JSON 往返；线上是新加的取值时落到 `Other`，原样带回来。
    #[test]
    fn ends_round_trip_and_unknown_ones_carry_their_raw_value() {
        for (end, json) in [
            (AgentRunEnd::Cancelled, "cancelled"),
            (AgentRunEnd::Uncertain, "uncertain"),
        ] {
            assert_eq!(serde_json::to_value(&end).unwrap(), serde_json::json!(json));
            assert_eq!(
                serde_json::from_value::<AgentRunEnd>(serde_json::json!(json)).unwrap(),
                end
            );
        }
        let other = AgentRunEnd::Other("aborted".into());
        assert_eq!(
            serde_json::from_value::<AgentRunEnd>(serde_json::json!("aborted")).unwrap(),
            other
        );
        assert_eq!(
            serde_json::to_value(&other).unwrap(),
            serde_json::json!("aborted")
        );
    }

    /// 契约：以反应回答的操作能经 JSON 往返，壳发回的键和状态行对得上。
    #[test]
    fn answer_choice_action_round_trips_through_json() {
        let action = crate::Action::AnswerAgentChoice {
            room_id: "!room:home.example".into(),
            item_id: "item-1".into(),
            key: "allow".into(),
        };
        assert_eq!(
            serde_json::from_value::<crate::Action>(serde_json::json!({
                "type": "answer_agent_choice",
                "room_id": "!room:home.example",
                "item_id": "item-1",
                "key": "allow"
            }))
            .unwrap(),
            action
        );
        assert_eq!(
            serde_json::from_str::<crate::Action>(&serde_json::to_string(&action).unwrap())
                .unwrap(),
            action
        );
    }

    /// 契约：AgentRun 条目线上是 `type: agent_run`，字段按蛇形命名，可省的取值是 null。
    #[test]
    fn agent_run_item_has_a_stable_json_contract() {
        let item = crate::TimelineItem {
            id: "9".into(),
            content: crate::TimelineContent::AgentRun {
                run: Some("$req".into()),
                state: AgentRunState::Working,
                step: None,
                steps: Some(2),
                progress: Some(0.25),
                result: None,
                end: None,
                body: "正在执行命令（第 2 步）".into(),
                card_id: None,
                choices: Vec::new(),
                sender_name: "Hermes".into(),
            },
        };
        let value = serde_json::to_value(&item).unwrap();
        assert_eq!(value["content"]["type"], "agent_run");
        assert_eq!(value["content"]["run"], "$req");
        assert_eq!(value["content"]["state"], "working");
        assert_eq!(value["content"]["steps"], 2);
        assert_eq!(value["content"]["progress"], 0.25);
        assert_eq!(value["content"]["card_id"], serde_json::Value::Null);
        assert_eq!(value["content"]["choices"], serde_json::json!([]));
        assert_eq!(value["content"]["sender_name"], "Hermes");
    }
}
