//! 已停止插件的启动接口：容器启动、就绪和核对完成后才返回成功。容器不在（被删了、Docker 数据丢了）
//! 就按安装记录照安装的路径重建再启动；网络不在也重建。做不了的按原因报类别（见 `InstallError`）：
//! 镜像拉不到 `image_pull_failed`，安装记录读不出 `record_unusable`。

use wonton_protocol::PermissionItem;

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};

use super::api::{current_permitted_requester, install_error, languages, requester};
use super::{Gateway, TunnelPeer, permissions};
use crate::plugin::{InstallError, PluginId};

pub async fn start(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::InstallApps)
    {
        return denied;
    }
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(error) => return install_error(&InstallError::Invalid(error.to_string())),
    };
    gateway.note_languages(peer.map(|Extension(peer)| peer), &headers);
    let result = gateway.plugins.start_detached(id.clone()).await;
    if let Err(denied) = current_permitted_requester(
        &gateway,
        peer,
        &headers,
        &person,
        PermissionItem::InstallApps,
    ) {
        return *denied;
    }
    if let Err(error) = result {
        return install_error(&error);
    }
    match gateway.plugins.installed_plugin(&id, &languages(&headers)) {
        Some(installed) => super::bot_projection::installed_response(
            &gateway,
            peer.map(|Extension(peer)| peer),
            &headers,
            &person,
            installed,
            StatusCode::OK,
        ),
        None => install_error(&InstallError::not_installed(&languages(&headers), &id)),
    }
}

pub async fn lifecycle(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::InstallApps)
    {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(error) => return install_error(&InstallError::Invalid(error.to_string())),
    };
    gateway.note_languages(peer.map(|Extension(peer)| peer), &headers);
    match gateway.plugins.lifecycle_status(&id) {
        Ok(lifecycle) => Json(lifecycle).into_response(),
        Err(error) => install_error(&error),
    }
}

pub async fn stop(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::InstallApps)
    {
        return denied;
    }
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(error) => return install_error(&InstallError::Invalid(error.to_string())),
    };
    gateway.note_languages(peer.map(|Extension(peer)| peer), &headers);
    let result = gateway.plugins.stop_detached(id.clone()).await;
    if let Err(denied) = current_permitted_requester(
        &gateway,
        peer,
        &headers,
        &person,
        PermissionItem::InstallApps,
    ) {
        return *denied;
    }
    if let Err(error) = result {
        return install_error(&error);
    }
    match gateway.plugins.installed_plugin(&id, &languages(&headers)) {
        Some(installed) => super::bot_projection::installed_response(
            &gateway,
            peer.map(|Extension(peer)| peer),
            &headers,
            &person,
            installed,
            StatusCode::OK,
        ),
        None => install_error(&InstallError::not_installed(&languages(&headers), &id)),
    }
}
