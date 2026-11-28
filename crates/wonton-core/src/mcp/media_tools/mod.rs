//! 核心给 Agent 的媒体工具（ADR-0113 第 5 节）。这里是只读的 `wonton__media_status`、`wonton__media_sessions`
//! 和改任务的 `wonton__media_task`；要设备动手的工具（开播、控制、备份、下载、删离线副本）走命令通道，另在
//! 别的模块。
//!
//! 工具作用于这个 Bot 的数据主体（[`subject`]），只看、只改这个人的任务和会话。都不发卡片：查是只读，任务的
//! 暂停、继续、取消、重试可撤（ADR-0024）；改任务仍受 AI 动手封顶，封顶只读时被拒。

mod present;
mod subject;

use std::sync::Arc;

use rmcp::model::{JsonObject, Tool};
use serde_json::{Value, json};
use wonton_protocol::AiLevel;
use wonton_protocol::media::tools::{MEDIA_SESSIONS_TOOL, MEDIA_STATUS_TOOL, MEDIA_TASK_TOOL};
use wonton_protocol::media::{MediaFailure, MediaFailureCode, PauseReason, TaskId, TaskKind};

use super::core_tools::tool;
use super::{Bus, Caller, ToolFailure, naming};
use crate::media::Media;
use crate::media::service_error::service_failure;
use crate::plugin::media_source::failure;
use subject::{Subject, subject};

/// 状态里最多列这么多条任务、这么多条会话，从新到旧；多的只报条数。Agent 要的是近况，整本历史会把上下文撑爆。
const STATUS_TASKS: usize = 50;
const STATUS_SESSIONS: usize = 10;

/// 总线上接的媒体服务和设备登记表。
pub(super) struct MediaDesk {
    pub(super) media: Arc<Media>,
    pub(super) devices: Arc<crate::device::Devices>,
}

/// 去掉 `wonton__` 前缀的工具名：核心工具在总线里按这个分派。
fn short(full: &'static str) -> &'static str {
    naming::split(full).map_or(full, |(_, tool)| tool)
}

pub(super) fn handles(tool: &str) -> bool {
    [MEDIA_STATUS_TOOL, MEDIA_SESSIONS_TOOL, MEDIA_TASK_TOOL]
        .into_iter()
        .any(|full| short(full) == tool)
}

pub(super) fn definitions() -> Vec<Tool> {
    let person = json!({"type": "string", "description": "替谁查：只能是这一轮说话的人（私人 Bot 只能是归属人），省略就是他"});
    vec![
        tool(
            short(MEDIA_STATUS_TOOL),
            "查一个人的上传、下载任务和播放会话：状态、进度、失败原因。从新到旧最多列 50 条任务、10 条会话。",
            json!({"type": "object", "properties": {
                "kind": {"type": "string", "enum": ["upload", "download", "playback"], "description": "只看这一种；省略看全部"},
                "person": person,
            }}),
            true,
        ),
        tool(
            short(MEDIA_SESSIONS_TOOL),
            "列出一个人的设备和每台设备上的播放会话。要叫哪台设备放、下载，从这里取设备标识。",
            json!({"type": "object", "properties": {"person": person}}),
            true,
        ),
        tool(
            short(MEDIA_TASK_TOOL),
            "暂停、继续、取消或重试这个人的一个任务。重试只对已结束的任务，回重试出来的那条新任务；同一个任务\
             重试多次回的是同一条。",
            json!({"type": "object", "properties": {
                "task": {"type": "string", "description": "任务 ID，从 wonton__media_status 取"},
                "op": {"type": "string", "enum": ["pause", "resume", "cancel", "retry"]},
            }, "required": ["task", "op"]}),
            false,
        ),
    ]
}

pub(super) fn call(
    bus: &Bus,
    caller: &Caller,
    tool: &str,
    args: &JsonObject,
) -> Result<Value, ToolFailure> {
    let desk = bus.media.get().ok_or_else(|| {
        media_failure(&failure(
            MediaFailureCode::PluginUnavailable,
            "这台服务器没有接上媒体服务",
        ))
    })?;
    let who = subject(bus, caller, text(args, "person")?)?;
    tracing::info!(plugin = caller.id.as_str(), tool, person = %who.person, "媒体工具替这个人办事");
    if tool == short(MEDIA_STATUS_TOOL) {
        status(desk, &who, text(args, "kind")?)
    } else if tool == short(MEDIA_SESSIONS_TOOL) {
        sessions(desk, &who)
    } else {
        if bus.ai_cap(caller) == AiLevel::ReadOnly {
            return Err(bus.read_only_refusal(caller, "不能改任务"));
        }
        task(desk, &who, args)
    }
}

fn status(desk: &MediaDesk, who: &Subject, kind: Option<&str>) -> Result<Value, ToolFailure> {
    // 外层 `None` 是都看；`Some(None)` 是只看会话。
    let kind = kind
        .map(|raw| match raw {
            "upload" => Ok(Some(TaskKind::Upload)),
            "download" => Ok(Some(TaskKind::Download)),
            "playback" => Ok(None),
            _ => Err(invalid(format!(
                "kind 只能是 upload、download 或 playback，不是 {raw}"
            ))),
        })
        .transpose()?;
    let playback_only = kind == Some(None);
    let store = &desk.media.store;
    let tasks = if playback_only {
        Vec::new()
    } else {
        store
            .tasks(
                Some(&who.person),
                kind.as_ref().and_then(Option::as_ref),
                None,
            )
            .map_err(|e| media_failure(&service_failure(&e)))?
    };
    let sessions = if matches!(kind, Some(Some(_))) {
        Vec::new()
    } else {
        store
            .sessions(Some(&who.person))
            .map_err(|e| media_failure(&service_failure(&e)))?
    };
    Ok(json!({
        "person": who.person,
        "tasks": newest(&tasks, STATUS_TASKS).map(present::task).collect::<Vec<_>>(),
        "more_tasks": tasks.len().saturating_sub(STATUS_TASKS),
        "sessions": newest(&sessions, STATUS_SESSIONS).map(present::session).collect::<Vec<_>>(),
        "more_sessions": sessions.len().saturating_sub(STATUS_SESSIONS),
    }))
}

fn sessions(desk: &MediaDesk, who: &Subject) -> Result<Value, ToolFailure> {
    let sessions = desk
        .media
        .store
        .sessions(Some(&who.person))
        .map_err(|e| media_failure(&service_failure(&e)))?;
    let devices = who.devices(&desk.devices);
    let listed: Vec<Value> = devices
        .iter()
        .map(|device| {
            let label = crate::files::device_label(&device.id);
            let active = sessions
                .iter()
                .filter(|s| s.device == label && s.stop_reason.is_none())
                .map(present::session)
                .collect::<Vec<_>>();
            json!({"device": label, "name": device.name, "sessions": active})
        })
        .collect();
    Ok(json!({"person": who.person, "devices": listed}))
}

fn task(desk: &MediaDesk, who: &Subject, args: &JsonObject) -> Result<Value, ToolFailure> {
    let raw = text(args, "task")?.ok_or_else(|| invalid("缺 task"))?;
    let id: TaskId = raw
        .parse()
        .map_err(|_| invalid(format!("task 不是任务 ID：{raw}")))?;
    let media = &desk.media;
    // 别人的任务当没有：不让 Agent 从回答里知道别人有什么。
    let gone = || {
        media_failure(&failure(
            MediaFailureCode::MediaGone,
            format!("{} 没有任务 {raw}", who.person),
        ))
    };
    media
        .store
        .task(&who.person, id)
        .map_err(|e| media_failure(&service_failure(&e)))?
        .ok_or_else(gone)?;
    let fold = |e: crate::media::TaskServiceError| media_failure(&service_failure(&e));
    let after = match text(args, "op")? {
        Some("pause") => media.tasks.pause(id, PauseReason::Agent).map(|()| id),
        Some("resume") => media.tasks.resume(id).map(|()| id),
        Some("cancel") => media.tasks.cancel(id).map(|()| id),
        Some("retry") => media.tasks.retry(id).map(|task| task.id),
        Some(other) => {
            return Err(invalid(format!(
                "op 只能是 pause、resume、cancel 或 retry，不是 {other}"
            )));
        }
        None => return Err(invalid("缺 op")),
    }
    .map_err(fold)?;
    let task = media
        .store
        .task(&who.person, after)
        .map_err(|e| media_failure(&service_failure(&e)))?
        .ok_or_else(gone)?;
    Ok(present::task(&task))
}

/// 从新到旧取前 `limit` 条：库里按建的先后存。
fn newest<T>(all: &[T], limit: usize) -> impl Iterator<Item = &T> {
    all.iter().rev().take(limit)
}

fn text<'a>(args: &'a JsonObject, key: &str) -> Result<Option<&'a str>, ToolFailure> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.as_str())),
        Some(_) => Err(invalid(format!("{key} 要是字符串"))),
    }
}

fn invalid(message: impl Into<String>) -> ToolFailure {
    ToolFailure::new("invalid_input", message)
}

/// 媒体的失败按信封的码报，原文照给。
fn media_failure(failure: &MediaFailure) -> ToolFailure {
    use MediaFailureCode as C;
    let kind = match &failure.code {
        C::LocalSourceUnavailable => "local_source_unavailable",
        C::LocalPermissionDenied => "local_permission_denied",
        C::LocalStorageFull => "local_storage_full",
        C::LocalIo => "local_io",
        C::TransportUnavailable => "transport_unavailable",
        C::Protocol => "protocol",
        C::PluginUnavailable => "plugin_unavailable",
        C::UpstreamError => "upstream_error",
        C::AuthenticationRequired => "authentication_required",
        C::UnsupportedByDevice => "unsupported_by_device",
        C::RateLimited => "rate_limited",
        C::MediaGone => "media_gone",
        C::ServerStorageFull => "server_storage_full",
        C::PermissionDenied => "permission_denied",
        C::SourceRequired => "source_required",
        C::StaleRevision => "stale_revision",
        C::SuspendedBySystem => "suspended_by_system",
        // 提供方回的新码：类别认不出，原码写进原文。
        C::Other(code) => {
            return ToolFailure::new("upstream_error", format!("{code}：{}", failure.message));
        }
    };
    ToolFailure::new(kind, failure.message.clone())
}
