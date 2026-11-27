//! 配额 `/quota`、`/offline`（ADR-0113“配额与清理”，M16）：读两本账；设备报自己的离线副本一共占多少。
//! 记在哪个人、哪台设备由配对的设备身份定，请求里没有这两项。

use axum::Extension;
use axum::body::Bytes;
use axum::response::{IntoResponse, Json, Response};
use wonton_protocol::media::MediaFailureCode;
use wonton_protocol::media::api::OfflineUsage;

use super::{Caller, reply, service_failure};
use crate::plugin::media_source::failure;

pub(super) async fn read(Extension(caller): Extension<Caller>) -> Response {
    match caller.media.store.quota(&caller.person) {
        Ok(view) => Json(view).into_response(),
        Err(error) => reply(&service_failure(&error)),
    }
}

pub(super) async fn report_offline(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let usage: OfflineUsage = match serde_json::from_slice(&body) {
        Ok(usage) => usage,
        Err(e) => {
            let message = format!("请求体不是约定的形状：{e}");
            return reply(&failure(MediaFailureCode::Protocol, message));
        }
    };
    let reported = caller
        .media
        .store
        .report_offline(&caller.person, &caller.device, usage.bytes);
    match reported {
        Ok(view) => Json(view).into_response(),
        Err(error) => reply(&service_failure(&error)),
    }
}
