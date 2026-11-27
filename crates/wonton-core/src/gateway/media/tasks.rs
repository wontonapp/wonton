//! 任务：建、列、读，暂停/继续/取消/重试，设备报进度、报写完、报失败。

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query};
use axum::response::{IntoResponse, Json, Response};
use hyper::StatusCode;
use wonton_protocol::media::api::{PauseRequest, TaskList, TaskProgressReport, TaskQuery};
use wonton_protocol::media::{
    MediaFailure, MediaFailureCode, MediaTask, PauseReason, Progress, ProgressUnit, TaskCreate,
    TaskId, TaskKind, TaskState, TaskTarget, new_task_id,
};

use super::{Caller, reply, service_failure};
use crate::media::session_service::unix_now_ms;
use crate::plugin::media_source::failure;

type Reply<T> = Result<T, MediaFailure>;

fn respond<T: serde::Serialize>(result: Reply<T>) -> Response {
    match result {
        Ok(body) => Json(body).into_response(),
        Err(failure) => reply(&failure),
    }
}

pub(super) fn parse_task(raw: &str) -> Reply<TaskId> {
    raw.parse()
        .map_err(|_| failure(MediaFailureCode::MediaGone, format!("没有任务 {raw}")))
}

/// 这个人的任务；别人的当没有。
pub(super) fn own_task(caller: &Caller, task: TaskId) -> Reply<MediaTask> {
    caller
        .media
        .store
        .task(&caller.person, task)
        .map_err(|e| service_failure(&e))?
        .ok_or_else(|| failure(MediaFailureCode::MediaGone, format!("没有任务 {task}")))
}

fn json<T: serde::de::DeserializeOwned>(body: &Bytes) -> Reply<T> {
    serde_json::from_slice(body).map_err(|e| {
        failure(
            MediaFailureCode::Protocol,
            format!("请求体不是约定的形状：{e}"),
        )
    })
}

pub(super) async fn create(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let made = (|| {
        let create: TaskCreate = json(&body)?;
        let person = caller.person(&create.person)?;
        let device = caller.device(&create.device)?;
        let progress = match &create.target {
            TaskTarget::Upload(items) => Progress {
                done: 0,
                total: Some(items.iter().map(|item| item.size).sum()),
                unit: ProgressUnit::Bytes,
            },
            _ => Progress {
                done: 0,
                total: None,
                unit: ProgressUnit::Bytes,
            },
        };
        let task = MediaTask {
            id: new_task_id(),
            kind: create.kind,
            person,
            device,
            target: create.target,
            state: TaskState::Queued,
            progress,
            retry_count: 0,
            retry_of: None,
            next_attempt_at: None,
            created_at: unix_now_ms(),
            partial_failures: Vec::new(),
        };
        if matches!(task.kind, TaskKind::Other(_)) || matches!(task.target, TaskTarget::Other(_)) {
            return Err(failure(
                MediaFailureCode::Protocol,
                "这一版核心不认识这种任务",
            ));
        }
        let store = &caller.media.store;
        store.create_task(&task).map_err(|e| service_failure(&e))?;
        // 上传点了名就先记下那一家；能不能用、在不在跑，开第一项时按记下的查。
        if let (TaskTarget::Upload(_), Some(source)) = (&task.target, &create.options.source) {
            store
                .bind_task_provider(&task.person, task.id, source)
                .map_err(|e| service_failure(&e))?;
        }
        Ok(task)
    })();
    match made {
        Ok(task) => (StatusCode::CREATED, Json(task)).into_response(),
        Err(failure) => reply(&failure),
    }
}

pub(super) async fn list(
    Extension(caller): Extension<Caller>,
    Query(query): Query<TaskQuery>,
) -> Response {
    respond((|| {
        let person = caller.person(query.person.as_deref().unwrap_or_default())?;
        // 库按状态的标签名筛，`Other` 的标签名就是原文，拿它带过去。
        let state = query.state.map(TaskState::Other);
        let tasks = caller
            .media
            .store
            .tasks(Some(&person), query.kind.as_ref(), state.as_ref())
            .map_err(|e| service_failure(&e))?;
        Ok(TaskList {
            tasks,
            cursor: None,
        })
    })())
}

pub(super) async fn read(Extension(caller): Extension<Caller>, Path(id): Path<String>) -> Response {
    respond(parse_task(&id).and_then(|task| own_task(&caller, task)))
}

/// `pause`、`resume`、`cancel`、`retry`，设备报的 `progress`、`complete`、`failure`。
pub(super) async fn act(
    Extension(caller): Extension<Caller>,
    Path((id, action)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let acted = (|| {
        let task = own_task(&caller, parse_task(&id)?)?;
        let tasks = &caller.media.tasks;
        let changed = match action.as_str() {
            "pause" => {
                let reason = if body.is_empty() {
                    PauseReason::User
                } else {
                    json::<PauseRequest>(&body)?.reason
                };
                tasks.pause(task.id, reason).map(|()| None)
            }
            "resume" => tasks.resume(task.id).map(|()| None),
            "cancel" => tasks.cancel(task.id).map(|()| None),
            "retry" => tasks.retry(task.id).map(Some),
            "progress" => {
                let report: TaskProgressReport = json(&body)?;
                tasks
                    .report_progress(task.id, report.done, report.total)
                    .map(Some)
            }
            "complete" => {
                if task.kind != TaskKind::Download {
                    return Err(failure(
                        MediaFailureCode::Protocol,
                        "只有下载任务由设备报写完",
                    ));
                }
                tasks.complete(task.id).map(Some)
            }
            "failure" => {
                let reported: MediaFailure = json(&body)?;
                tasks.fail(task.id, reported).map(|_| None)
            }
            other => {
                return Err(failure(
                    MediaFailureCode::Protocol,
                    format!("任务上没有 {other} 这个动作"),
                ));
            }
        }
        .map_err(|e| service_failure(&e))?;
        match changed {
            Some(task) => Ok(task),
            None => own_task(&caller, task.id),
        }
    })();
    respond(acted)
}

/// 上传的一项在设备那边失败：这一项标失败，其余照常。
pub(super) async fn item_failure(
    Extension(caller): Extension<Caller>,
    Path((id, item)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    respond((|| {
        let task = own_task(&caller, parse_task(&id)?)?;
        let reported: MediaFailure = json(&body)?;
        caller
            .media
            .tasks
            .fail_item(task.id, &item, reported)
            .map_err(|e| service_failure(&e))
    })())
}
