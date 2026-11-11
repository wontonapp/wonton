//! 已停止插件的启动接口：容器启动、就绪和核对完成后才返回成功。容器不在（被删了、Docker 数据丢了）
//! 就按安装记录照安装的路径重建再启动；网络不在也重建。做不了的按原因报类别（见 `InstallError`）：
//! 镜像拉不到 `image_pull_failed`，安装记录读不出 `record_unusable`。

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::HeaderMap;

use super::api::{install_denied, install_error, languages};
use super::{Gateway, TunnelPeer};
use crate::plugin::{InstallError, PluginId};

pub async fn start(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = install_denied(&gateway, peer, &headers) {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(error) => return install_error(&InstallError::Invalid(error.to_string())),
    };
    gateway.note_languages(peer.map(|Extension(peer)| peer), &headers);
    if let Err(error) = gateway.plugins.start_detached(id.clone()).await {
        return install_error(&error);
    }
    match gateway.plugins.installed_plugin(&id, &languages(&headers)) {
        Some(installed) => Json(installed).into_response(),
        None => install_error(&InstallError::not_installed(&languages(&headers), &id)),
    }
}
