//! 编排器的任务事件（`_wonton/task`，ADR-0059）：把任务开成话题、把状态写到工作板和话题里。
//!
//! 第一次见到某个任务 id：在主对话发一行“新任务「标题」”，拿它当话题根，把任务的 ACP
//! 会话（`session/load`）绑到这个话题上。之后这个会话的回复、工具调用都进话题；状态变化
//! 只更新工作板和话题里的一行通知——要人行动的（等你回答、等你审阅、受阻、失败）提及
//! 主人，其余发 `m.notice` 不推送。任务话题不受活跃话题上限管，也不注入别人的动态、
//! 不做改文件排队（ADR-0059 第 2 节）。

use serde_json::Value;
use wonton_l10n::{Arg, Lang};

use super::{Room, RoomState, TurnContext, turns};
use crate::error::{Detail, Error, Result};
use crate::l10n::text;
use crate::state::TaskRecord;
use crate::topics::{SessionKey, Topics};

/// 任务状态里要人行动的几种：在话题里提及主人（推送）。
const ACTIONABLE: [&str; 4] = ["needs_answer", "needs_review", "blocked", "failed"];

struct TaskEvent {
    id: String,
    title: String,
    state: String,
    session_id: String,
}

fn parse(params: &Value) -> Result<TaskEvent> {
    let task = &params["task"];
    let text = |value: &Value, name: &str| {
        value[name]
            .as_str()
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| Error::InvalidInput(Detail::data(format!("_wonton/task 缺少 {name}"))))
    };
    let state = text(task, "state")?;
    Ok(TaskEvent {
        id: text(task, "id")?,
        title: text(task, "title")?,
        state,
        session_id: text(task, "taskSessionId")?,
    })
}

pub(super) async fn handle<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    params: &Value,
) -> Result<()> {
    let event = parse(params)?;
    let record = match context.bridge.sessions.task(&event.id).await {
        Some(record) => record,
        None => {
            open(context, state, topics, &event).await?;
            return notice(context, topics, &event).await;
        }
    };
    // 套接的会话变了（重试成新会话）时换绑。
    if record.session_id != event.session_id {
        state
            .sessions
            .bind(&SessionKey::Topic(record.root.clone()), &event.session_id);
        context
            .bridge
            .sessions
            .put_task(
                &event.id,
                TaskRecord {
                    title: event.title.clone(),
                    root: record.root.clone(),
                    session_id: event.session_id.clone(),
                },
            )
            .await?;
    }
    notice(context, topics, &event).await
}

/// 第一次见到任务：开话题根、登记、绑会话（之后的 `ensure_session` 走 `session/load`）。
async fn open<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    event: &TaskEvent,
) -> Result<()> {
    let line = text(
        context.lang,
        "task-new",
        &[("title", Arg::from(event.title.as_str()))],
    );
    let root = context.room.send_text_event(&line).await?;
    context
        .bridge
        .sessions
        .put_task(
            &event.id,
            TaskRecord {
                title: event.title.clone(),
                root: root.clone(),
                session_id: event.session_id.clone(),
            },
        )
        .await?;
    let key = SessionKey::Topic(root);
    state.sessions.bind(&key, &event.session_id);
    let session = state.sessions.state(&key);
    session.session_id = Some(event.session_id.clone());
    session.first_turn = false;
    context
        .bridge
        .sessions
        .set(
            &context.room.room_id(),
            &key,
            crate::state::SessionRecord {
                session_id: event.session_id.clone(),
                cwd: context.bridge.config.workspace.clone(),
                mode: context.bridge.config.mode.clone(),
            },
        )
        .await?;
    topics
        .turn_started_task(&key, &event.title, super::now())
        .await?;
    Ok(())
}

/// 状态写进工作板；要人行动的提及主人，其余只发 notice。
async fn notice<R: Room>(
    context: &TurnContext<R>,
    topics: &mut Topics,
    event: &TaskEvent,
) -> Result<()> {
    let key = SessionKey::Topic(
        context
            .bridge
            .sessions
            .task(&event.id)
            .await
            .map(|task| task.root)
            .unwrap_or_default(),
    );
    topics.task_state(&key, &event.state, super::now()).await?;
    let Some(SessionKey::Topic(root)) = Some(&key) else {
        return Ok(());
    };
    if root.is_empty() {
        return Ok(());
    }
    let line = state_text(event, context.lang);
    if ACTIONABLE.contains(&event.state.as_str()) {
        context
            .room
            .send_thread_mention(root, &line, &context.bridge.config.owner)
            .await?;
    } else {
        context.room.send_thread_notice(root, &line).await?;
    }
    if event.state == "done" {
        let _ = topics.end(&key, super::now(), context.lang).await;
        if let Some(session_id) = state_session(context, event).await {
            let _ = context.bridge.acp.close(&session_id).await;
        }
        // 会话记录一起删：完成的任务再来通知只更新工作板，不再拿死会话去 load。
        let _ = context
            .bridge
            .sessions
            .remove(&context.room.room_id(), &key)
            .await;
        let _ = turns::send_to(
            context,
            &SessionKey::Main,
            &text(
                context.lang,
                "task-done",
                &[("title", Arg::from(event.title.as_str()))],
            ),
        )
        .await;
    }
    Ok(())
}

async fn state_session<R: Room>(context: &TurnContext<R>, event: &TaskEvent) -> Option<String> {
    context
        .bridge
        .sessions
        .task(&event.id)
        .await
        .map(|task| task.session_id)
}

fn state_text(event: &TaskEvent, lang: Lang) -> String {
    let id = match event.state.as_str() {
        "working" => "task-working",
        "needs_answer" => "task-needs-answer",
        "needs_review" => "task-needs-review",
        "blocked" => "task-blocked",
        "failed" => "task-failed",
        "done" => "task-done-state",
        other => {
            return text(
                lang,
                "task-other",
                &[
                    ("title", Arg::from(event.title.as_str())),
                    ("state", Arg::from(other)),
                ],
            );
        }
    };
    text(lang, id, &[("title", Arg::from(event.title.as_str()))])
}
