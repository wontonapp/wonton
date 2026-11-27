//! 队列 `/queue`（ADR-0113 第 4 节，M10）：读，和带版本的改动。版本对不上回 409，体是当前队列
//! （不是失败信封），客户端按它重放。改成了就推给正在播的那家（[`push`]）。

use std::sync::Arc;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Json, Response};
use hyper::StatusCode;
use wonton_protocol::media::api::QueueChange;
use wonton_protocol::media::provider::QueueSnapshot;
use wonton_protocol::media::{MediaFailure, MediaFailureCode, MediaItemRef, Queue, QueueOp};

use super::sessions::session_route;
use super::{Caller, reply, service_failure};
use crate::gateway::Gateway;
use crate::media::session_service::QueueChanged;
use crate::plugin::media_source::failure;

pub(super) async fn read(
    State(_): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
) -> Response {
    match caller.media.sessions.queue(&caller.person) {
        Ok(queue) => Json(queue).into_response(),
        Err(error) => reply(&service_failure(&error)),
    }
}

pub(super) async fn change(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    body: Bytes,
) -> Response {
    let change: QueueChange = match serde_json::from_slice(&body) {
        Ok(change) => change,
        Err(e) => {
            let message = format!("请求体不是约定的形状：{e}");
            return reply(&failure(MediaFailureCode::Protocol, message));
        }
    };
    let read_only = matches!(change.op, QueueOp::Read);
    let changed = caller
        .media
        .sessions
        .change_queue(&caller.person, change.revision, change.op);
    match changed {
        Ok(QueueChanged::Applied(queue)) => {
            if !read_only {
                push(&gateway, &caller, &queue).await;
            }
            Json(queue).into_response()
        }
        Ok(QueueChanged::Stale(queue)) => (StatusCode::CONFLICT, Json(queue)).into_response(),
        Err(error) => reply(&service_failure(&error)),
    }
}

/// 把队列推给这个人正在播的那条会话的提供方（`PUT /playback/{session}/queue`，`savePlayQueue` 的翻法）。
/// 只带那一家的条目（来源是它或没写来源的）；正在放的不是它家的就不带 `current`。没有在播的会话就不推。
/// 推不过去只记日志：队列的事实源在核心，设备侧的改动照样算成。
pub(super) async fn push(gateway: &Gateway, caller: &Caller, queue: &Queue) {
    let pushed = async {
        let sessions = caller
            .media
            .store
            .sessions(Some(&caller.person))
            .map_err(|e| service_failure(&e))?;
        let Some(session) = sessions.into_iter().find(|s| s.stop_reason.is_none()) else {
            return Ok(());
        };
        let Some(plan) = &session.plan else {
            return Ok(());
        };
        let route = session_route(gateway, caller, &session).await?;
        let theirs = |entry: &MediaItemRef| {
            entry
                .source
                .as_deref()
                .is_none_or(|source| source == route.provider)
        };
        let current = queue
            .cursor
            .filter(|&at| queue.entries.get(at).is_some_and(theirs))
            .map(|at| queue.entries[..at].iter().filter(|e| theirs(e)).count());
        let snapshot = QueueSnapshot {
            entries: queue
                .entries
                .iter()
                .filter(|e| theirs(e))
                .cloned()
                .collect(),
            current,
            position_ms: session.position_ms,
            mode: queue.mode.clone(),
        };
        route
            .client
            .save_queue(&session.person, &plan.session, &snapshot)
            .await
    };
    let pushed: Result<(), MediaFailure> = pushed.await;
    if let Err(e) = pushed {
        tracing::warn!(person = %caller.person, code = ?e.code, error = %e.message, "队列没推给提供方");
    }
}
