//! 命令通道 `/commands`（ADR-0113 第 5 节，M13）：设备长轮询取发给自己的命令，做完回执。
//! 只能取、回执发给发请求的这台设备的命令。

use axum::Extension;
use axum::body::Bytes;
use axum::extract::Path;
use axum::response::{IntoResponse, Json, Response};
use hyper::StatusCode;
use wonton_protocol::media::api::{CommandAck, CommandList, CommandQuery};
use wonton_protocol::media::{MediaFailureCode, TaskId};

use super::{Caller, MediaQuery, reply, service_failure};
use crate::plugin::media_source::failure;

pub(super) async fn poll(
    Extension(caller): Extension<Caller>,
    MediaQuery(query): MediaQuery<CommandQuery>,
) -> Response {
    let device = match caller.device(&query.device) {
        Ok(device) => device,
        Err(failure) => return reply(&failure),
    };
    match caller.media.commands.poll(&device, query.after).await {
        Ok(commands) if commands.is_empty() => StatusCode::NO_CONTENT.into_response(),
        Ok(commands) => Json(CommandList { commands }).into_response(),
        Err(error) => reply(&service_failure(&error)),
    }
}

pub(super) async fn ack(
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let Ok(command) = id.parse::<TaskId>() else {
        return reply(&failure(
            MediaFailureCode::Protocol,
            format!("{id} 不是命令号"),
        ));
    };
    let ack: CommandAck = match serde_json::from_slice(&body) {
        Ok(ack) => ack,
        Err(e) => {
            let message = format!("请求体不是约定的形状：{e}");
            return reply(&failure(MediaFailureCode::Protocol, message));
        }
    };
    match caller.media.commands.ack(&caller.device, command, &ack) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => reply(&service_failure(&error)),
    }
}
