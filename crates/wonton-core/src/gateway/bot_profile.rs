use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use serde_json::json;
use wonton_protocol::BotProfileUpdate;

use super::api::error_response;
use super::{Gateway, TunnelPeer, isolation};
use crate::plugin::PluginId;
use crate::plugin::bot_profile::ProfileError;

pub(super) async fn save(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    match gateway.device_person(peer.map(|Extension(peer)| peer)) {
        None => {
            return error_response(
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "需要经隧道连接的主人设备",
            );
        }
        // Bot 的名字和头像全家都看得到，由主人定（ADR-0045“谁能做什么”）。
        Some(person) if !person.is_owner() => {
            return error_response(
                StatusCode::FORBIDDEN,
                "permission_required",
                "Bot 资料只有主人能改",
            );
        }
        Some(_) => {}
    }
    if !isolation::not_from_another_page(&headers, &gateway.core_origin()) {
        return error_response(
            StatusCode::FORBIDDEN,
            "cross_site",
            "资料接口不接受其他网页发来的请求",
        );
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(error) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &error.to_string());
        }
    };
    let update = match serde_json::from_slice::<BotProfileUpdate>(&body) {
        Ok(update) => update,
        Err(error) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &error.to_string());
        }
    };
    let updated =
        tokio::spawn(async move { gateway.plugins.update_bot_profile(&id, update).await }).await;
    let updated = match updated {
        Ok(updated) => updated,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                &format!("修改 Bot 资料的流程意外中止：{error}"),
            );
        }
    };
    match updated {
        Ok(updated) => Json(updated).into_response(),
        Err(error) => failure(error),
    }
}

fn failure(error: ProfileError) -> Response {
    let status = match error {
        ProfileError::NotFound => StatusCode::NOT_FOUND,
        ProfileError::Name(_) | ProfileError::Media(_) | ProfileError::InvalidInput(_) => {
            StatusCode::BAD_REQUEST
        }
        ProfileError::MatrixUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        ProfileError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        ProfileError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
    };
    let problem = match &error {
        ProfileError::Name(problem) => Some(json!(problem)),
        ProfileError::Media(problem) => Some(json!(problem)),
        _ => None,
    };
    let mut response = error_response(status, error.kind(), &error.to_string());
    if let Some(problem) = problem {
        *response.body_mut() = axum::body::Body::from(
            json!({"error": {
                "kind": error.kind(), "message": error.to_string(), "problem": problem,
            }})
            .to_string(),
        );
    }
    response
}
