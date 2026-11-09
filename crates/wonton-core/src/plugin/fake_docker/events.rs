//! 假 Docker 的 `/events` 端点：订阅广播，把每条事件按一行 JSON 写回（Docker 也是按行流的 JSON）。

use std::convert::Infallible;

use axum::body::{Body, Bytes};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use tokio::sync::broadcast;

pub(super) fn events_response(receiver: broadcast::Receiver<Value>) -> Response {
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        match receiver.recv().await {
            Ok(value) => {
                let mut line = serde_json::to_vec(&value).expect("事件可序列化");
                line.push(b'\n');
                Some((Ok::<_, Infallible>(Bytes::from(line)), receiver))
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                Some((Ok::<_, Infallible>(Bytes::new()), receiver))
            }
            Err(broadcast::error::RecvError::Closed) => None,
        }
    });
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Body::from_stream(stream),
    )
        .into_response()
}
