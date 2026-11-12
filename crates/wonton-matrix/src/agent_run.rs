//! Agent 进度状态消息（MSC4564，ADR-0082）：从消息内容里解析状态对象，变成
//! [`crate::Message::agent_run`]。解析只认格式，不管发送者是谁；只对 Bot 账号解读在客户端核心做。

use serde_json::{Map, Value};

use wonton_protocol::{AgentRunChoice, AgentRunEnd, AgentRunState};

/// MSC4564 稳定前的键，桥现在发它。
pub(crate) const FLUXCHAT_FIELD: &str = "org.fluxchat.agent";
/// MSC4564 稳定后的键，两个都有时以它为准。
pub(crate) const MSC_FIELD: &str = "m.agent";
/// 我们加的结束原因，放在状态对象旁边（ADR-0082 第 1 节）。
pub(crate) const END_FIELD: &str = "app.wonton.end";

/// 解析出来的一轮状态，字段是发送者的自述，原样带上。`body` 不在这里：它就是消息本身的正文，
/// 客户端从消息的 [`crate::MessageBody`] 里取。
#[derive(Clone, Debug, PartialEq)]
pub struct AgentRunStatus {
    /// 这一轮针对的请求消息的事件 ID（`run`）。
    pub run: Option<String>,
    pub state: AgentRunState,
    /// 当前步骤的一句话（`step`）。
    pub step: Option<String>,
    /// 到现在完成的步数（`steps`）。
    pub steps: Option<u32>,
    /// 进度 `0.0..=1.0`（`progress`）。
    pub progress: Option<f32>,
    /// 结束时的一句话总结（`result`）。
    pub result: Option<String>,
    /// 怎么结束的（`app.wonton.end`）。
    pub end: Option<AgentRunEnd>,
    /// Bot 自己发的待回答选项（`choices`）。
    pub choices: Vec<AgentRunChoice>,
}

/// 从消息内容解析状态对象；不是状态消息（没有对象、`v` 不是 1、没有 `state`）返回 `None`，按普通消息显示。
pub(crate) fn parse(content: &Map<String, Value>) -> Option<AgentRunStatus> {
    let object = content
        .get(MSC_FIELD)
        .or_else(|| content.get(FLUXCHAT_FIELD))?
        .as_object()?;
    if object.get("v").and_then(Value::as_i64) != Some(1) {
        return None;
    }
    let state = object.get("state")?.as_str()?;
    Some(AgentRunStatus {
        run: string(object, "run"),
        state: serde_json::from_value(Value::from(state))
            .unwrap_or(AgentRunState::Other(state.to_owned())),
        step: string(object, "step"),
        steps: object
            .get("steps")
            .and_then(Value::as_u64)
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX)),
        progress: object
            .get("progress")
            .and_then(Value::as_f64)
            .map(|p| p as f32),
        result: string(object, "result"),
        end: content.get(END_FIELD).and_then(Value::as_str).map(|raw| {
            serde_json::from_value::<AgentRunEnd>(Value::from(raw))
                .unwrap_or(AgentRunEnd::Other(raw.to_owned()))
        }),
        choices: choices(object),
    })
}

fn string(object: &Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

/// `choices` 是 `{key, label}` 的数组；缺字段或类型不对的选项跳过，整个不是数组就当没有。
fn choices(object: &Map<String, Value>) -> Vec<AgentRunChoice> {
    object
        .get("choices")
        .and_then(Value::as_array)
        .map(|choices| {
            choices
                .iter()
                .filter_map(|choice| {
                    Some(AgentRunChoice {
                        key: string(choice.as_object()?, "key")?,
                        label: string(choice.as_object()?, "label")?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_value(value: Value) -> Option<AgentRunStatus> {
        parse(value.as_object().expect("内容要是对象"))
    }

    #[test]
    fn a_status_object_parses_with_all_fields() {
        let status = parse_value(json!({
            "msgtype": "m.notice",
            "body": "正在执行命令（第 2 步）",
            MSC_FIELD: {
                "v": 1,
                "run": "$request",
                "state": "working",
                "step": "执行命令",
                "steps": 2,
                "progress": 0.5,
            },
        }))
        .expect("是状态消息");
        assert_eq!(status.state, AgentRunState::Working);
        assert_eq!(status.run.as_deref(), Some("$request"));
        assert_eq!(status.step.as_deref(), Some("执行命令"));
        assert_eq!(status.steps, Some(2));
        assert_eq!(status.progress, Some(0.5));
        assert_eq!(status.end, None);
        assert!(status.choices.is_empty());
    }

    #[test]
    fn m_agent_wins_over_the_fluxchat_key_and_end_comes_from_the_sibling_key() {
        let status = parse_value(json!({
            "body": "等你确认",
            FLUXCHAT_FIELD: {"v": 1, "state": "working"},
            MSC_FIELD: {"v": 1, "state": "waiting"},
            END_FIELD: "cancelled",
        }))
        .expect("是状态消息");
        assert_eq!(status.state, AgentRunState::Waiting);
        assert_eq!(status.end, Some(AgentRunEnd::Cancelled));
    }

    #[test]
    fn unknown_state_and_end_keep_their_raw_value() {
        let status = parse_value(json!({
            "body": "x",
            MSC_FIELD: {"v": 1, "state": "brewing"},
            END_FIELD: "aborted",
        }))
        .expect("是状态消息");
        assert_eq!(status.state, AgentRunState::Other("brewing".into()));
        assert_eq!(status.end, Some(AgentRunEnd::Other("aborted".into())));
    }

    #[test]
    fn anything_that_is_not_a_status_object_is_none() {
        // v 不是 1（包括缺了）、state 缺了、对象不是对象，都按普通消息。
        for content in [
            json!({"body": "x", MSC_FIELD: {"v": 2, "state": "working"}}),
            json!({"body": "x", MSC_FIELD: {"state": "working"}}),
            json!({"body": "x", MSC_FIELD: {"v": 1}}),
            json!({"body": "x", MSC_FIELD: "working"}),
            json!({"body": "没有状态对象"}),
        ] {
            assert_eq!(parse_value(content), None, "要按普通消息显示");
        }
    }

    #[test]
    fn choices_read_key_and_label_and_skip_broken_ones() {
        let status = parse_value(json!({
            "body": "怎么继续？",
            MSC_FIELD: {
                "v": 1,
                "state": "waiting",
                "choices": [
                    {"key": "allow", "label": "继续"},
                    {"key": "deny"},
                    "allow",
                ],
            },
        }))
        .expect("是状态消息");
        assert_eq!(
            status.choices,
            vec![AgentRunChoice {
                key: "allow".into(),
                label: "继续".into()
            }]
        );
    }
}
