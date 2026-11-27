//! 队列 `/queue`（ADR-0113 第 4 节，M10）：读，和带版本的改动。版本对不上回 409，体是当前队列
//! （不是失败信封），客户端按它重放。

use std::sync::Arc;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Json, Response};
use hyper::StatusCode;
use wonton_protocol::media::MediaFailureCode;
use wonton_protocol::media::api::QueueChange;

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
    State(_): State<Arc<Gateway>>,
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
    let changed = caller
        .media
        .sessions
        .change_queue(&caller.person, change.revision, change.op);
    match changed {
        Ok(QueueChanged::Applied(queue)) => Json(queue).into_response(),
        Ok(QueueChanged::Stale(queue)) => (StatusCode::CONFLICT, Json(queue)).into_response(),
        Err(error) => reply(&service_failure(&error)),
    }
}
