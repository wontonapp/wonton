//! 不放行公开房间（ADR-0045 的“网关管房间规则”）：联邦关着，公开房间只多一条绕过邀请的路。设备和 Bot 两个入口
//! 都管这几类请求：
//!
//! - `POST /_matrix/client/{版本}/createRoom`：带 `visibility: public`、`preset: public_chat`，或在
//!   `initial_state` 里带公开的 join_rules；
//! - `PUT /_matrix/client/{版本}/rooms/{房间}/state/m.room.join_rules`（状态键为空）：把 join_rules 改成
//!   `public`。
//!
//! 都回 403 `M_FORBIDDEN`，`x-wonton-error` 写 [`GatewayRefusal::PublicRoom`]。只为判断读一遍请求体，放行时把
//! 读出来的原始字节原样装回转给 Tuwunel；判断用的请求体有大小上限，超了回 413。别的请求原封不动。

use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::response::Response;
use hyper::header::{CONTENT_LENGTH, HeaderValue, TRANSFER_ENCODING};
use hyper::{Method, StatusCode};
use serde_json::Value;
use wonton_protocol::GatewayRefusal;

use super::matrix::{matrix_error, percent_decode, refusal};

const JOIN_RULES: &str = "m.room.join_rules";

/// 判断用的请求体上限：正常的 createRoom 只有几 KB（房间名、话题、邀请名单），放宽到 256 KiB。
const MAX_BODY: usize = 256 * 1024;

/// 要按“公开房间”检查的请求：建房，或把房间的 join_rules 改成 public。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    CreateRoom,
    JoinRules,
}

/// 命中才读请求体；路径按字面切段，事件类型和状态键百分号解码后比对。
fn target(method: &Method, path: &str) -> Option<Target> {
    if !(*method == Method::POST || *method == Method::PUT) {
        return None;
    }
    let rest = path.strip_prefix("/_matrix/client/")?;
    let mut segments = rest.split('/');
    // 版本段（v3、r0）非空就行。
    if segments.next().is_none_or(|version| version.is_empty()) {
        return None;
    }
    if *method == Method::POST {
        // createRoom：就这一段，后面没有别的。
        return (segments.next() == Some("createRoom") && segments.next().is_none())
            .then_some(Target::CreateRoom);
    }
    (segments.next() == Some("rooms")
        && segments.next().is_some_and(|room| !room.is_empty())
        && segments.next() == Some("state")
        && text(segments.next()).is_some_and(|kind| kind == JOIN_RULES)
        // join_rules 的状态键为空：带非空状态键的是另一条状态事件，改不到房间的 join_rules。
        && segments
            .next()
            .is_none_or(|key| text(Some(key)).is_some_and(|k| k.is_empty()))
        && segments.next().is_none())
    .then_some(Target::JoinRules)
}

/// 百分号解码一段路径，解出来不是 UTF-8 就当没有（不是合法的事件类型或状态键）。
fn text(segment: Option<&str>) -> Option<String> {
    segment
        .map(percent_decode)
        .and_then(|decoded| String::from_utf8(decoded).ok())
}

/// createRoom 的公开写法：`visibility: public`、`preset: public_chat`，或在 `initial_state` 里带公开的
/// join_rules。
fn create_room_is_public(body: &Value) -> bool {
    body.get("visibility").and_then(Value::as_str) == Some("public")
        || body.get("preset").and_then(Value::as_str) == Some("public_chat")
        || body
            .get("initial_state")
            .and_then(Value::as_array)
            .is_some_and(|events| {
                events.iter().any(|event| {
                    event.get("type").and_then(Value::as_str) == Some(JOIN_RULES)
                        && event.pointer("/content/join_rule").and_then(Value::as_str)
                            == Some("public")
                })
            })
}

/// 把 join_rules 改成 public 的状态事件体：状态接口的请求体就是事件 content 本身。
fn join_rules_is_public(body: &Value) -> bool {
    body.get("join_rule").and_then(Value::as_str) == Some("public")
}

/// 检查要建（改）公开房间的请求；放行时把读出来的请求体原样装回，不放行时给出要回的响应。
pub(super) async fn check(req: Request) -> Result<Request, Box<Response>> {
    let Some(target) = target(req.method(), req.uri().path()) else {
        return Ok(req);
    };
    let too_large = || {
        Box::new(matrix_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "M_TOO_LARGE",
            "invalid_input",
            "请求体太大，检查不了要不要放行公开房间",
        ))
    };
    let bad_json = || {
        Box::new(matrix_error(
            StatusCode::BAD_REQUEST,
            "M_BAD_JSON",
            "invalid_input",
            "请求体不是 JSON",
        ))
    };
    let (mut parts, body) = req.into_parts();
    let bytes = to_bytes(body, MAX_BODY).await.map_err(|_| too_large())?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| bad_json())?;
    let public = match target {
        Target::CreateRoom => create_room_is_public(&body),
        Target::JoinRules => join_rules_is_public(&body),
    };
    if public {
        return Err(Box::new(refusal(
            GatewayRefusal::PublicRoom,
            "这台服务器不放行公开房间",
        )));
    }
    // 转发读出来的这一份：请求体已经读掉了，装回原始字节，上游看到的就是客户端发来的那一份。
    parts.headers.remove(TRANSFER_ENCODING);
    parts
        .headers
        .insert(CONTENT_LENGTH, HeaderValue::from(bytes.len()));
    Ok(Request::from_parts(parts, Body::from(bytes)))
}
