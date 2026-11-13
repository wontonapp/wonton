//! 一轮一条的 Agent 进度状态消息（ADR-0082，MSC4564）：输入 ACP 事件，决定发什么、
//! 编辑成什么。状态消息是带 `org.fluxchat.agent` 对象的 `m.notice`，之后每步用
//! `m.replace` 编辑推进，以 `done` 或 `failed` 收尾；话题里的轮次挂同一话题，主对话里
//! 的不挂话题关系。
//!
//! 编辑合并（ADR-0082）：状态值变化立刻编辑，只有步骤变化每 3 秒最多一次，一轮最多
//! 60 次编辑、到了以后只等最终那次。状态消息的事件 ID 落盘在桥数据目录
//! `run-status/<房间>.json`；桥重启时把还在 `working`/`waiting` 的都编辑成
//! `failed` + `app.wonton.end: uncertain`，不留一条永远在干活中的消息。
//!
//! `step` 只按工具类别写（ADR-0082）：标题是 Agent 给的，可能带命令参数、路径或密钥，
//! 一律不进聊天。

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonton_l10n::{Arg, Lang};

use super::Room;
use crate::{
    acp::PromptResult,
    error::{Detail, Error, Result, format_error},
    l10n::{CATALOG, text},
    topics::SessionKey,
};

/// 只有步骤变化的编辑最多每 3 秒一次（ADR-0082）。
const STEP_MERGE: Duration = Duration::from_secs(3);
/// 一轮最多 60 次编辑，到了以后只等最终那次。
const EDIT_MAX: u32 = 60;
/// 状态对象的结构版本（MSC4564 的 `v`）。
const FORMAT_VERSION: u32 = 1;
/// 状态对象的键（MSC4564 稳定前的前缀）。
const STATUS_KEY: &str = "org.fluxchat.agent";
/// 状态对象旁边标结束原因的键，只取 `cancelled`、`uncertain`（ADR-0082）。
const END_KEY: &str = "app.wonton.end";

/// 一轮的进行状态（MSC4564 的四个值）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum State {
    Working,
    Waiting,
    Done,
    Failed,
}

impl State {
    fn wire(self) -> &'static str {
        match self {
            State::Working => "working",
            State::Waiting => "waiting",
            State::Done => "done",
            State::Failed => "failed",
        }
    }
}

/// 桥加的结束原因：打断了、结果不确定（别的客户端只看到 `failed`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    Cancelled,
    Uncertain,
}

impl End {
    fn wire(self) -> &'static str {
        match self {
            End::Cancelled => "cancelled",
            End::Uncertain => "uncertain",
        }
    }
}

/// 步骤的类别，文字按房间语言从类别表取（ADR-0082 第 3 节）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Thinking,
    Replying,
    Tool(ToolKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Fetch,
    Think,
    Other,
}

impl ToolKind {
    fn from_wire(kind: Option<&str>) -> Self {
        match kind {
            Some("read") => ToolKind::Read,
            Some("edit") => ToolKind::Edit,
            Some("delete") => ToolKind::Delete,
            Some("move") => ToolKind::Move,
            Some("search") => ToolKind::Search,
            Some("execute") => ToolKind::Execute,
            Some("fetch") => ToolKind::Fetch,
            Some("think") => ToolKind::Think,
            _ => ToolKind::Other,
        }
    }

    fn text_id(self) -> &'static str {
        match self {
            ToolKind::Read => "run-tool-read",
            ToolKind::Edit => "run-tool-edit",
            ToolKind::Delete => "run-tool-delete",
            ToolKind::Move => "run-tool-move",
            ToolKind::Search => "run-tool-search",
            ToolKind::Execute => "run-tool-execute",
            ToolKind::Fetch => "run-tool-fetch",
            ToolKind::Think => "run-thinking",
            ToolKind::Other => "run-tool-other",
        }
    }
}

impl Step {
    fn text_id(self) -> &'static str {
        match self {
            Step::Thinking => "run-thinking",
            Step::Replying => "run-replying",
            Step::Tool(kind) => kind.text_id(),
        }
    }
}

/// 落盘的一条进行中的一轮：重启后按它把遗留状态编辑成不确定。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    key: SessionKey,
    /// 这一轮针对的请求消息事件 ID（`org.fluxchat.agent.run`）。
    request: String,
    /// 状态消息的事件 ID。
    event_id: String,
    /// 话题根；主对话的轮次是 `None`。
    thread: Option<String>,
    /// 房间语言的目录标签，恢复编辑时用它写字。
    lang: String,
}

/// 进行中的一轮的运行态。只有进行中的轮次在表里；编辑成终态并发出后即移除。
struct Run {
    entry: Entry,
    state: State,
    end: Option<End>,
    step: Option<Step>,
    steps: u32,
    progress: Option<f64>,
    /// 失败原因的整句（按房间语言）；进行中为 `None`。
    failure: Option<String>,
    /// 状态值变了，下一次 flush 立刻编辑。
    dirty_state: bool,
    /// 步骤（类别、步数、进度）变了，按 3 秒合并编辑。
    dirty_step: bool,
    /// 终态的那次编辑，不受合并和 60 次上限管。
    final_edit: bool,
    last_edit: Option<Instant>,
    edits: u32,
}

impl Run {
    fn lang(&self) -> Lang {
        CATALOG.negotiate(&[self.entry.lang.as_str()])
    }

    fn body(&self, lang: Lang) -> String {
        match self.state {
            State::Working => match self.step {
                Some(step) => text(lang, step.text_id(), &[]),
                None => text(lang, "run-thinking", &[]),
            },
            State::Waiting => text(lang, "run-waiting", &[]),
            State::Done | State::Failed => self.result(lang),
        }
    }

    /// 结束时的一句话总结（MSC4564 的 `result`）。
    fn result(&self, lang: Lang) -> String {
        match (self.state, self.end) {
            (State::Done, _) => {
                if self.steps > 0 {
                    text(
                        lang,
                        "run-done-steps",
                        &[("steps", Arg::from(self.steps.to_string()))],
                    )
                } else {
                    text(lang, "run-done-plain", &[])
                }
            }
            (State::Failed, Some(End::Cancelled)) => text(lang, "run-cancelled", &[]),
            (State::Failed, Some(End::Uncertain)) => self
                .failure
                .clone()
                .unwrap_or_else(|| text(lang, "run-uncertain", &[])),
            (State::Failed, None) => self.failure.clone().unwrap_or_default(),
            (State::Working | State::Waiting, _) => String::new(),
        }
    }

    /// 状态消息的完整内容：外层带对象（不认识编辑的客户端看到最新 `body`），编辑时
    /// `m.new_content` 里带同样的对象（MSC4564），`m.relates_to` 带编辑关系。
    fn envelope(&self) -> Value {
        let lang = self.lang();
        let mut object = json!({
            "v": FORMAT_VERSION,
            "run": self.entry.request,
            "state": self.state.wire(),
        });
        if matches!(self.state, State::Working | State::Waiting) {
            if let Some(step) = self.step {
                object["step"] = json!(text(lang, step.text_id(), &[]));
            }
            if let Some(progress) = self.progress {
                object["progress"] = json!(progress);
            }
        }
        object["steps"] = json!(self.steps);
        if matches!(self.state, State::Done | State::Failed) {
            let result = self.result(lang);
            object["result"] = json!(result);
        }
        let mut content = json!({
            "msgtype": "m.notice",
            "body": self.body(lang),
            STATUS_KEY: object,
        });
        if let Some(end) = self.end {
            content[END_KEY] = json!(end.wire());
        }
        if let Some(root) = &self.entry.thread {
            // 话题回退指向根：状态消息是桥在话题里发出的一条，不需要真的回复谁。
            content["m.relates_to"] = json!({
                "rel_type": "m.thread",
                "event_id": root,
                "is_falling_back": true,
                "m.in_reply_to": {"event_id": root},
            });
        }
        content
    }

    fn replacement(&self) -> Value {
        let mut content = self.envelope();
        let new_content = content
            .as_object()
            .map(|object| {
                object
                    .iter()
                    .filter(|(name, _)| !name.starts_with("m."))
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect::<serde_json::Map<String, Value>>()
            })
            .unwrap_or_default();
        content["m.new_content"] = Value::Object(new_content);
        content["m.relates_to"] = json!({
            "rel_type": "m.replace",
            "event_id": self.entry.event_id,
        });
        content
    }
}

/// 一轮的结果按“答完了”的口径（ADR-0082）：`end_turn` 收尾、没截断、没有权限失败。
/// 旁聊结果行同用这个口径；打断（cancelled）算“失败或被打断”。
fn answered_prompt(result: &PromptResult) -> bool {
    !result.output_limited && result.permission_error.is_none() && result.stop_reason == "end_turn"
}

/// 状态消息和旁聊结果行（ADR-0093）共用的口径；出错的一轮没答完。
pub(super) fn answered(result: &Result<PromptResult>) -> bool {
    matches!(result, Ok(result) if answered_prompt(result))
}

/// 一个房间的进行中的一轮。一个会话同一时间只有一轮（桥保证），按会话键各记一条。
#[derive(Default)]
pub(super) struct Board {
    file: PathBuf,
    runs: Vec<Run>,
}

impl Board {
    /// 打开落盘文件（没有就先不建，第一次写时建）。
    pub(super) async fn open(data_dir: PathBuf, room_id: &str) -> Self {
        let file = data_dir.join("run-status").join(format!("{room_id}.json"));
        let runs = match tokio::fs::read(&file).await {
            Ok(bytes) => serde_json::from_slice::<Vec<Entry>>(&bytes)
                .map(|entries| {
                    entries
                        .into_iter()
                        .map(|entry| Run {
                            state: State::Working,
                            end: None,
                            step: None,
                            steps: 0,
                            progress: None,
                            failure: None,
                            dirty_state: false,
                            dirty_step: false,
                            final_edit: false,
                            last_edit: None,
                            edits: 0,
                            entry,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        Self { file, runs }
    }

    /// 桥重启：遗留的 `working`/`waiting` 状态消息全部编辑成不确定（ADR-0082）。
    /// 编辑是尽力而为：失败也照常清掉落盘记录，这轮已经不在了，重启后会再显示成
    /// 不确定的机会由下次重启兜底。
    pub(super) async fn recover<R: Room>(&mut self, room: &R) {
        for run in &mut self.runs {
            run.state = State::Failed;
            run.end = Some(End::Uncertain);
            run.step = None;
            run.failure = Some(text(run.lang(), "run-bridge-restart", &[]));
            let content = run.replacement();
            if let Err(error) = room.replace_run_status(content).await {
                tracing::warn!(error = %error, event_id = run.entry.event_id, "恢复遗留状态消息失败");
            } else {
                tracing::info!(
                    event_id = run.entry.event_id,
                    request = run.entry.request,
                    "重启恢复：遗留状态编辑成 uncertain"
                );
            }
        }
        self.runs.clear();
        self.persist().await;
    }

    /// 还在进行中的轮数；断线收尾要等它们全部落定。
    pub(super) fn busy(&self) -> bool {
        self.runs
            .iter()
            .any(|run| matches!(run.state, State::Working | State::Waiting))
    }

    /// 一轮开始：发 `working` 状态消息，`run` 记请求消息的事件 ID。发不出就没有状态行，
    /// 这一轮照常跑（状态行是提示，答案和错误照发）。
    pub(super) async fn start<R: Room>(
        &mut self,
        room: &R,
        key: &SessionKey,
        request: &str,
        lang: Lang,
    ) {
        if self.runs.iter().any(|run| &run.entry.key == key) {
            return;
        }
        let entry = Entry {
            key: key.clone(),
            request: request.to_owned(),
            event_id: String::new(),
            thread: match key {
                SessionKey::Main => None,
                SessionKey::Topic(root) => Some(root.clone()),
            },
            lang: CATALOG.tag(lang).to_owned(),
        };
        let mut run = Run {
            entry,
            state: State::Working,
            end: None,
            step: Some(Step::Thinking),
            steps: 0,
            progress: None,
            failure: None,
            dirty_state: false,
            dirty_step: false,
            final_edit: false,
            last_edit: None,
            edits: 0,
        };
        match room.send_run_status(run.envelope()).await {
            Ok(event_id) => {
                run.entry.event_id = event_id;
                // 首次发出就是这一轮的第一次渲染：3 秒合并窗口从它算起。
                run.last_edit = Some(Instant::now());
                tracing::info!(
                    event_id = run.entry.event_id,
                    request = run.entry.request,
                    "状态消息已发出（working）"
                );
                self.runs.push(run);
                self.persist().await;
            }
            Err(error) => tracing::warn!(error = %error, "发状态消息失败，这一轮没有状态行"),
        }
    }

    /// 一条 ACP 更新：工具调用、思考块、回复块、计划。没有进行中的一轮就忽略。
    pub(super) async fn observe<R: Room>(&mut self, room: &R, key: &SessionKey, update: &Value) {
        let Some(run) = self.runs.iter_mut().find(|run| &run.entry.key == key) else {
            return;
        };
        match update["sessionUpdate"].as_str() {
            Some("tool_call") => {
                run.steps += 1;
                run.step = Some(Step::Tool(ToolKind::from_wire(update["kind"].as_str())));
                run.dirty_step = true;
            }
            Some("agent_thought_chunk") => {
                run.step = Some(Step::Thinking);
                run.dirty_step = true;
            }
            Some("agent_message_chunk") => {
                run.step = Some(Step::Replying);
                run.dirty_step = true;
            }
            Some("plan") => {
                let entries = update["entries"].as_array();
                let total = entries.map(|items| items.len()).unwrap_or(0);
                let done = entries
                    .map(|items| {
                        items
                            .iter()
                            .filter(|item| item["status"].as_str() == Some("completed"))
                            .count()
                    })
                    .unwrap_or(0);
                run.progress = (total > 0).then(|| done as f64 / total as f64);
                run.dirty_step = true;
            }
            _ => return,
        }
        self.flush(room, Instant::now()).await;
    }

    /// 权限请求变成核心卡片：等主人回答（ADR-0082）。
    pub(super) async fn waiting<R: Room>(&mut self, room: &R, key: &SessionKey) {
        let Some(run) = self.runs.iter_mut().find(|run| &run.entry.key == key) else {
            return;
        };
        if run.state != State::Waiting {
            run.state = State::Waiting;
            run.dirty_state = true;
        }
        self.flush(room, Instant::now()).await;
    }

    /// 主人回答了卡片（或 Agent 撤回了请求）：回到干活。
    pub(super) async fn resumed<R: Room>(&mut self, room: &R, key: &SessionKey) {
        let Some(run) = self.runs.iter_mut().find(|run| &run.entry.key == key) else {
            return;
        };
        if run.state == State::Waiting {
            run.state = State::Working;
            run.dirty_state = true;
        }
        self.flush(room, Instant::now()).await;
    }

    /// 一轮结束：按停止原因和错误类别收尾（ADR-0082 第 3 节）。编辑失败时留在表里，
    /// 由 flush 每 3 秒重试，直到发出或桥重启交给恢复。
    pub(super) async fn finish<R: Room>(
        &mut self,
        room: &R,
        key: &SessionKey,
        result: &Result<PromptResult>,
    ) {
        let Some(run) = self.runs.iter_mut().find(|run| &run.entry.key == key) else {
            return;
        };
        let lang = run.lang();
        let (state, end, failure) = match result {
            Err(error) => {
                let uncertain = matches!(
                    error,
                    Error::TransportUnavailable(Detail::Msg {
                        id: "acp-connection-closed",
                        ..
                    })
                ) && run.steps > 0;
                if uncertain {
                    (State::Failed, Some(End::Uncertain), None)
                } else {
                    (State::Failed, None, Some(format_error(error, lang)))
                }
            }
            Ok(result) if answered_prompt(result) => (State::Done, None, None),
            Ok(result) => {
                if result.output_limited {
                    (State::Failed, None, Some(text(lang, "output-limited", &[])))
                } else if let Some(detail) = &result.permission_error {
                    (
                        State::Failed,
                        None,
                        Some(text(
                            lang,
                            "permission-failed",
                            &[("message", Arg::from(detail.format(lang)))],
                        )),
                    )
                } else {
                    match result.stop_reason.as_str() {
                        "cancelled" => (State::Failed, Some(End::Cancelled), None),
                        "max_tokens" => {
                            (State::Failed, None, Some(text(lang, "run-max-tokens", &[])))
                        }
                        "max_turn_requests" => {
                            (State::Failed, None, Some(text(lang, "run-max-turns", &[])))
                        }
                        "refusal" => (State::Failed, None, Some(text(lang, "run-refusal", &[]))),
                        other => (
                            State::Failed,
                            None,
                            Some(text(
                                lang,
                                "run-failed-other",
                                &[("reason", Arg::from(other))],
                            )),
                        ),
                    }
                }
            }
        };
        run.state = state;
        run.end = end;
        run.failure = failure;
        run.step = None;
        run.progress = None;
        run.dirty_state = true;
        run.final_edit = true;
        self.flush(room, Instant::now()).await;
    }

    /// 把要发的编辑发出去：状态值变化立刻，只有步骤变化的按 3 秒合并，60 次上限到了
    /// 只放行终态那次。
    pub(super) async fn flush<R: Room>(&mut self, room: &R, now: Instant) {
        let mut removed = false;
        for index in (0..self.runs.len()).rev() {
            let run = &mut self.runs[index];
            let due = run.dirty_state
                || (run.dirty_step
                    && run
                        .last_edit
                        .is_none_or(|at| now.duration_since(at) >= STEP_MERGE));
            if !due || (run.edits >= EDIT_MAX && !run.final_edit) {
                continue;
            }
            let content = run.replacement();
            match room.replace_run_status(content).await {
                Ok(()) => {
                    let done = matches!(run.state, State::Done | State::Failed);
                    run.dirty_state = false;
                    run.dirty_step = false;
                    run.final_edit = false;
                    run.edits += 1;
                    run.last_edit = Some(now);
                    tracing::debug!(
                        event_id = run.entry.event_id,
                        state = run.state.wire(),
                        steps = run.steps,
                        edit = run.edits,
                        "状态消息已编辑"
                    );
                    if done {
                        self.runs.remove(index);
                        removed = true;
                    }
                }
                Err(error) => tracing::warn!(error = %error, "编辑状态消息失败"),
            }
        }
        if removed {
            self.persist().await;
        }
    }

    /// 把进行中的一轮落盘。只在发送和移除时写：编辑不改事件 ID。
    async fn persist(&self) {
        if let Some(parent) = self.file.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let entries: Vec<&Entry> = self.runs.iter().map(|run| &run.entry).collect();
        let Ok(bytes) = serde_json::to_vec_pretty(&entries) else {
            return;
        };
        let temp = self.file.with_extension("tmp");
        if tokio::fs::write(&temp, bytes).await.is_ok() {
            let _ = tokio::fs::rename(&temp, &self.file).await;
        }
    }
}
