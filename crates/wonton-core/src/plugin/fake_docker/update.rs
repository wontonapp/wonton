//! 假 Docker 的容器资源更新端点。

use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Value, json};

use super::{Resources, json_reply};

/// `POST /containers/{name}/update`：把资源字段的值合并进容器创建时的 `HostConfig`，
/// 之后 inspect 报告的就是新值（Docker 重建前 update 也是这样立刻生效）。
pub(super) fn update_container(docker: &mut Resources, name: &str, raw: &Bytes) -> Response {
    if let Some(message) = docker.update_error.take() {
        return json_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"message": message}),
        );
    }
    let body: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    let created = docker
        .created
        .entry(name.to_owned())
        .or_insert_with(|| json!({"HostConfig": {}}));
    let host = created["HostConfig"]
        .as_object_mut()
        .expect("容器的 HostConfig 是对象");
    for field in ["Memory", "MemorySwap", "MemoryReservation", "NanoCpus"] {
        if let Some(value) = body.get(field) {
            host.insert(field.to_owned(), value.clone());
        }
    }
    json_reply(StatusCode::OK, json!({"Warnings": []}))
}
