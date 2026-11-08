//! 应用商店 API（ADR-0017 的“接口：App 和核心”）。读商店和看来源要装卸权限：主人和“能装插件”的
//! 家人（[`api::install_denied`]），加删来源只给主人（[`api::owner_denied`]）。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/store?refresh=true` | 商店（`StoreListing`）；`refresh` 时官方索引、社区插件和已安装插件的新版本都不等缓存过期 |
//! | GET | `/_wonton/api/store/plugin?source=github:…` | 来源最新的版本的详情（`PluginDetail`） |
//! | POST | `/_wonton/api/store/sources` | 添加来源（`AddSourceRequest`），能读到才写下来（201 `AddedSource`） |
//! | DELETE | `/_wonton/api/store/sources?address=…` | 删除添加的来源，已安装的插件不动 |

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use serde::Deserialize;
use wonton_protocol::AddSourceRequest;

use super::Gateway;
use super::api::{install_denied, install_error, invalid_body, languages, owner_denied, requester};

#[derive(Deserialize)]
pub struct StoreQuery {
    #[serde(default)]
    refresh: bool,
}

pub async fn store(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<super::TunnelPeer>>,
    headers: HeaderMap,
    Query(query): Query<StoreQuery>,
) -> Response {
    if let Some(denied) = install_denied(&gateway, peer, &headers) {
        return denied;
    }
    drop(gateway.plugins.spawn_upgrade_check(query.refresh));
    let listing = gateway
        .plugins
        .store_listing(&languages(&headers), query.refresh)
        .await;
    Json(listing).into_response()
}

#[derive(Deserialize)]
pub struct StorePluginQuery {
    source: String,
}

pub async fn store_plugin(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<super::TunnelPeer>>,
    headers: HeaderMap,
    Query(query): Query<StorePluginQuery>,
) -> Response {
    if let Some(denied) = install_denied(&gateway, peer, &headers) {
        return denied;
    }
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    match gateway
        .plugins
        .store_plugin(&query.source, &languages(&headers))
        .await
    {
        Ok(mut detail) => {
            if !person.is_owner() {
                detail.scope = None;
            }
            Json(detail).into_response()
        }
        Err(e) => install_error(&e),
    }
}

pub async fn add_store_source(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<super::TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) = owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    let request = match serde_json::from_slice::<AddSourceRequest>(&body) {
        Ok(request) => request,
        Err(e) => return install_error(&invalid_body(&headers, e)),
    };
    match gateway
        .plugins
        .add_store_source(&request.address, &languages(&headers))
        .await
    {
        Ok(added) => (StatusCode::CREATED, Json(added)).into_response(),
        Err(e) => install_error(&e),
    }
}

#[derive(Deserialize)]
pub struct RemoveSourceQuery {
    address: String,
}

pub async fn remove_store_source(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<super::TunnelPeer>>,
    headers: HeaderMap,
    Query(query): Query<RemoveSourceQuery>,
) -> Response {
    if let Some(denied) = owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    match gateway
        .plugins
        .remove_store_source(&query.address, &languages(&headers))
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => install_error(&e),
    }
}
