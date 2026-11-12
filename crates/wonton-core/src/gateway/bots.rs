//! 共享 Bot 的接口（ADR-0045）：主人在 App 里建一个归全家的 Bot。私人 Bot 的建法（家人加入时）在别的
//! 任务里做，到时候走核心内部的 [`crate::plugin::PluginManager::add_bot`]。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::NewSharedBot;

use super::api::install_error;
use super::{Gateway, TunnelPeer, api, pages};

/// `POST SHARED_BOTS_PATH`：请求体 [`NewSharedBot`]，返回 [`wonton_protocol::BotRef`]。
pub async fn create_shared(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) = api::owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    let req: NewSharedBot = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(e) => {
            return pages::error(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &format!("请求体不是建共享 Bot 的参数：{e}"),
                &api::languages(&headers),
            );
        }
    };
    match gateway.plugins.create_shared_bot(&req).await {
        Ok(bot) => (StatusCode::CREATED, Json(bot)).into_response(),
        Err(e) => install_error(&e),
    }
}
