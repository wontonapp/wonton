//! 服务器配对的管理 API（ADR-0050）。只收主人设备经隧道来的请求：没有主人令牌的路径，配对是主人在
//! App 的“其他服务器”页上做的事（ADR-0017 的设备门）。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/peers` | 已配对和等对方同意的服务器，加还没过期的出码（`PeersView`） |
//! | POST | `/_wonton/api/peers/code` | 出一张新码，上一张作废（`PeerCode`） |
//! | POST | `/_wonton/api/peers` | 请求体是扫来的码（`PeerScan`）；连上出码方并记成“等对方同意” |
//! | DELETE | `/_wonton/api/peers/{server_name}` | 删一台服务器的配对，通知对方（尽力而为） |

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{PEER_ERROR_HEADER, PeerError, PeerErrorBody, PeerProblem, PeerScan};

use super::isolation;
use super::{Gateway, TunnelPeer, api};

use crate::plugin::languages::Languages;

/// 主人的语言，错误说明按它写（ADR-0043）。
fn languages(headers: &HeaderMap) -> Languages {
    let accept = headers
        .get(hyper::header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok());
    Languages::from_header(accept)
}

/// 配对接口的鉴权：只认经隧道的已配对设备。设备身份随隧道上的每个请求自动带上，插件网页发的请求
/// 不算（ADR-0017）：插件网页探测不到主人的服务器名单。
fn denied(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> Option<Response> {
    let Some(Extension(TunnelPeer(id))) = peer else {
        return Some(error(&languages(headers), PeerError::Unauthenticated));
    };
    if !gateway.tunnel.devices.contains(&id) {
        return Some(error(&languages(headers), PeerError::Unauthenticated));
    }
    if isolation::not_from_another_page(headers, &gateway.core_origin()) {
        None
    } else {
        Some(api::error_response(
            StatusCode::FORBIDDEN,
            "cross_site",
            "配对接口不接受其他网页发来的请求",
        ))
    }
}

/// 一个配对错误的标准回应：状态码按类别（`PeerError::http_status`），头 `X-Wonton-Error`，
/// 错误体 `PeerErrorBody`，说明按 App 的语言写（ADR-0043）。
fn error(langs: &Languages, kind: PeerError) -> Response {
    let status = kind
        .http_status()
        .and_then(|status| StatusCode::from_u16(status).ok())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = (
        status,
        Json(PeerErrorBody {
            error: PeerProblem {
                kind,
                message: message(kind, langs),
            },
        }),
    )
        .into_response();
    response.headers_mut().insert(
        PEER_ERROR_HEADER,
        hyper::header::HeaderValue::from_str(kind.as_str()).expect("类别头是合法的值"),
    );
    response
}

/// 类别的主人说得清的说明（ADR-0043）。
fn message(kind: PeerError, langs: &Languages) -> String {
    let id = match kind {
        PeerError::CodeExpired => "peers-error-code-expired",
        PeerError::CodeInvalid => "peers-error-code-invalid",
        PeerError::PeerUnreachable => "peers-error-peer-unreachable",
        PeerError::PeerRejected => "peers-error-peer-rejected",
        PeerError::AlreadyPaired => "peers-error-already-paired",
        PeerError::SelfPairing => "peers-error-self-pairing",
        PeerError::NotPaired => "peers-error-not-paired",
        PeerError::Unauthenticated => "peers-error-unauthenticated",
        PeerError::CodeRevoked => "peers-error-code-revoked",
        PeerError::TooManyRequests => "peers-error-too-many-requests",
        PeerError::RequestNotFound => "peers-error-request-not-found",
        // 客户端核心才有的类别和比这个 App 新的类别，核心不会回应。
        PeerError::TransportUnavailable | PeerError::Protocol | PeerError::Other => {
            "peers-error-transport"
        }
    };
    crate::l10n::text(langs, id, &[])
}

/// 没接 Matrix 的核心没有服务器配对：如实说不可用。
fn unavailable(langs: &Languages) -> Response {
    let body = Json(PeerErrorBody {
        error: PeerProblem {
            kind: PeerError::Other,
            message: crate::l10n::text(langs, "peers-error-matrix-unavailable", &[]),
        },
    });
    let mut response = (StatusCode::SERVICE_UNAVAILABLE, body).into_response();
    response.headers_mut().insert(
        PEER_ERROR_HEADER,
        hyper::header::HeaderValue::from_static("other"),
    );
    response
}

fn peers(gateway: &Gateway) -> Option<&Arc<crate::peers::Peers>> {
    gateway.peers.as_ref()
}

/// `GET /_wonton/api/peers`
pub async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = denied(&gateway, peer, &headers) {
        return denied;
    }
    // 记下主人的语言：配对请求卡片的文字从它来（照插件管理接口的做法）。
    gateway.plugins.note_owner_languages(
        headers
            .get(hyper::header::ACCEPT_LANGUAGE)
            .and_then(|v| v.to_str().ok()),
    );
    let langs = languages(&headers);
    let Some(peers) = peers(&gateway) else {
        return unavailable(&langs);
    };
    Json(peers.view(&langs)).into_response()
}

/// `POST /_wonton/api/peers/code`
pub async fn code(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = denied(&gateway, peer, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    let Some(peers) = peers(&gateway) else {
        return unavailable(&langs);
    };
    Json(peers.issue_code()).into_response()
}

/// `POST /_wonton/api/peers`
pub async fn scan(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) = denied(&gateway, peer, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    let Some(peers) = peers(&gateway) else {
        return unavailable(&langs);
    };
    let request: PeerScan = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => return error(&langs, PeerError::CodeInvalid),
    };
    match peers.scan(request.code.expose()).await {
        Ok(()) => Json(serde_json::json!({"status": "awaiting_approval"})).into_response(),
        Err(kind) => error(&langs, kind),
    }
}

/// `DELETE /_wonton/api/peers/{server_name}`
pub async fn remove(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(server_name): Path<String>,
) -> Response {
    if let Some(denied) = denied(&gateway, peer, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    let Some(peers) = peers(&gateway) else {
        return unavailable(&langs);
    };
    match peers.remove(server_name.trim()).await {
        Ok(true) => Json(serde_json::json!({"removed": true})).into_response(),
        Ok(false) => api::error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            &crate::l10n::text(&langs, "peers-error-not-paired-remove", &[]),
        ),
        Err(e) => {
            tracing::error!(error = %e, "配对记录写不进去");
            api::error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage", &e.to_string())
        }
    }
}
