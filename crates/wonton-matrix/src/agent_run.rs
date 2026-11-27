//! Agent 进度状态消息（MSC4564，ADR-0082）：从消息内容里解析状态对象和旁边的工具调用清单（ADR-0128），变成
//! [`crate::Message::agent_run`]。解析只认格式，不管发送者是谁；只对 Bot 账号解读在客户端核心做。

use serde_json::{Map, Value};

use wonton_protocol::{AgentRunChoice, AgentRunEnd, AgentRunState, AgentToolCall, ChoiceSend};

/// MSC4564 稳定前的键，桥现在发它。
pub(crate) const FLUXCHAT_FIELD: &str = "org.fluxchat.agent";
/// MSC4564 稳定后的键，两个都有时以它为准。
pub(crate) const MSC_FIELD: &str = "m.agent";
/// 我们加的结束原因，放在状态对象旁边（ADR-0082 第 1 节）。
pub(crate) const END_FIELD: &str = "app.wonton.end";
/// 我们加的工具调用清单，也放在状态对象旁边（ADR-0128 第 2 节）。
pub(crate) const TRACE_FIELD: &str = "app.wonton.trace";

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
    /// 这一轮调过的工具（`app.wonton.trace` 的 `calls`），按 Bot 给的顺序。
    pub trace: Vec<AgentToolCall>,
}

impl AgentRunStatus {
    /// 按这条状态消息上的反应把我点过的选项对回来：加过的是选上了，在发、发失败的照实记。
    /// 只认本账号的反应，别人的反应是 Bot 放的选项本身。
    pub(crate) fn mark_answers(&mut self, reactions: &[crate::Reaction]) {
        for choice in &mut self.choices {
            let own = reactions
                .iter()
                .find(|reaction| reaction.key == choice.key)
                .and_then(|reaction| reaction.own.as_ref());
            match own {
                Some(crate::OwnReaction::Sent) => choice.chosen = true,
                Some(crate::OwnReaction::Sending) => choice.send = ChoiceSend::Sending,
                Some(crate::OwnReaction::Failed { error, .. }) => {
                    choice.send = ChoiceSend::Failed {
                        message: error.clone(),
                    };
                }
                None => {}
            }
        }
    }
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
        trace: trace(content),
    })
}

/// `app.wonton.trace` 的 `calls`：`v` 不是 1 或 `calls` 不是数组就当没有清单；缺字段、类型不对的项跳过，
/// 不认识的 `kind`、`status` 落到 `Other`。
fn trace(content: &Map<String, Value>) -> Vec<AgentToolCall> {
    let Some(trace) = content.get(TRACE_FIELD).and_then(Value::as_object) else {
        return Vec::new();
    };
    if trace.get("v").and_then(Value::as_i64) != Some(1) {
        return Vec::new();
    }
    trace
        .get("calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| serde_json::from_value(call.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
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
                        chosen: false,
                        send: ChoiceSend::Idle,
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
    use wonton_protocol::{AgentToolKind, AgentToolStatus};

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
    fn the_trace_beside_the_status_object_parses_in_order() {
        let status = parse_value(json!({
            "body": "正在执行命令（第 2 步）",
            FLUXCHAT_FIELD: {"v": 1, "state": "working", "steps": 2},
            TRACE_FIELD: {
                "v": 1,
                "calls": [
                    {"id": "c1", "kind": "read", "title": "read_file: notes.md", "status": "completed"},
                    {"id": "c2", "kind": "execute", "title": "terminal: ls", "status": "in_progress"},
                ],
            },
        }))
        .expect("是状态消息");
        assert_eq!(
            status.trace,
            vec![
                AgentToolCall {
                    id: "c1".into(),
                    kind: AgentToolKind::Read,
                    title: "read_file: notes.md".into(),
                    status: AgentToolStatus::Completed,
                },
                AgentToolCall {
                    id: "c2".into(),
                    kind: AgentToolKind::Execute,
                    title: "terminal: ls".into(),
                    status: AgentToolStatus::InProgress,
                },
            ]
        );
    }

    /// Hermes v0.23.4 加 Hermes 插件 1.4.0 在私聊里真发出的一次编辑（`m.new_content`，2026-11-27 开发环境实测）。
    #[test]
    fn what_the_hermes_plugin_sends_parses() {
        let status = parse_value(json!({
            "msgtype": "m.notice",
            "body": "执行命令（第 2 步）",
            "org.fluxchat.agent": {
                "run": "$IWWLJQ0_PuN1sUqMOWCGYzIFFYoaafjDwJD61rkY-WI",
                "state": "working", "step": "执行命令", "steps": 2, "v": 1
            },
            "app.wonton.trace": {"calls": [
                {"id": "call_6119caa15746408aab099497", "kind": "execute", "status": "completed",
                 "title": "terminal: curl -s -o /dev/null -H \"Authorization: Bearer ***\" https://example.com"},
                {"id": "call_68a1a6946041478896f49df9", "kind": "execute", "status": "in_progress",
                 "title": "terminal: sleep 8 + 1 条命令"}
            ], "v": 1}
        }))
        .expect("是状态消息");
        assert_eq!(status.state, AgentRunState::Working);
        assert_eq!(status.steps, Some(2));
        let statuses: Vec<_> = status
            .trace
            .iter()
            .map(|call| call.status.clone())
            .collect();
        assert_eq!(
            statuses,
            [AgentToolStatus::Completed, AgentToolStatus::InProgress]
        );
        assert!(
            status
                .trace
                .iter()
                .all(|call| call.kind == AgentToolKind::Execute)
        );
    }

    #[test]
    fn broken_trace_items_are_skipped_and_new_values_fall_to_other() {
        let status = parse_value(json!({
            "body": "x",
            MSC_FIELD: {"v": 1, "state": "working"},
            TRACE_FIELD: {
                "v": 1,
                "calls": [
                    {"id": "c1", "kind": "switch_mode", "title": "plan", "status": "pending"},
                    {"id": "c2", "kind": "read", "status": "completed"},
                    {"id": "c3", "kind": "read", "title": 7, "status": "completed"},
                    "c4",
                ],
            },
        }))
        .expect("是状态消息");
        assert_eq!(
            status.trace,
            vec![AgentToolCall {
                id: "c1".into(),
                kind: AgentToolKind::Other("switch_mode".into()),
                title: "plan".into(),
                status: AgentToolStatus::Other("pending".into()),
            }]
        );
    }

    #[test]
    fn a_trace_of_another_version_or_shape_is_no_trace() {
        let call = json!({"id": "c1", "kind": "read", "title": "t", "status": "completed"});
        for trace in [
            json!({"v": 2, "calls": [call.clone()]}),
            json!({"calls": [call.clone()]}),
            json!({"v": 1, "calls": call.clone()}),
            json!([call.clone()]),
        ] {
            let status = parse_value(json!({
                "body": "x",
                MSC_FIELD: {"v": 1, "state": "working"},
                TRACE_FIELD: trace,
            }))
            .expect("状态对象本身是好的");
            assert!(status.trace.is_empty(), "要当没有清单");
        }
        // 没有状态对象时清单也不算：它只是状态消息的附带。
        assert_eq!(
            parse_value(json!({"body": "x", TRACE_FIELD: {"v": 1, "calls": [call]}})),
            None
        );
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
                label: "继续".into(),
                chosen: false,
                send: ChoiceSend::Idle,
            }]
        );
    }
}
