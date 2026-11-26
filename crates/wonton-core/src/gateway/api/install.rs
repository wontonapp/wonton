use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{InstallRequest, InstallSource, PermissionItem, UninstallRequest};

use super::api_switch::{enabled, granted, secrets, settings};
use super::{
    Gateway, InstallQuery, Person, TunnelPeer, UninstallQuery, comma_list, error_response,
    install_error, install_gate_checks, install_gates, invalid_body, json_body, languages, package,
    permissions, requester, response_for_requester,
};
use crate::plugin::languages::Languages;
use crate::plugin::{InstallError, PluginId, PluginPackage, PluginView};

pub async fn preview(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::InstallApps)
    {
        return denied;
    }
    let langs = languages(&headers);
    let preview = match json_body::<InstallSource>(&headers, &body) {
        Some(Err(e)) => return install_error(&e),
        Some(Ok(request)) => {
            gateway
                .plugins
                .preview_source(&request.source, &langs)
                .await
        }
        None => match package(&langs, &headers, &body) {
            Ok(package) => gateway.plugins.preview_upload(package, &langs).await,
            Err(e) => return install_error(&e),
        },
    };
    match preview {
        Ok(mut preview) => {
            if let Ok(person) = requester(&gateway, peer, &headers) {
                install_gates::project_preview(&gateway, &person, &mut preview);
            }
            Json(preview).into_response()
        }
        Err(e) => install_error(&e),
    }
}

pub async fn install(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Query(query): Query<InstallQuery>,
    body: Bytes,
) -> Response {
    let langs = languages(&headers);
    gateway.note_languages(peer.map(|Extension(peer)| peer), &headers);
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    // 不能装的人在读请求体之前就被挡下（ADR-0088 的权限项按人判断，与请求内容无关）。
    if let Some(denied) = install_gates::deny_check(&gateway, &person) {
        return denied;
    }
    // 门槛要预览的内容写卡片，先把请求体读出来；安装仍按预览给的来源与摘要执行。
    match json_body::<InstallRequest>(&headers, &body) {
        Some(Err(e)) => install_error(&e),
        Some(Ok(request)) => {
            install_by_source(gateway, peer, headers, person, request, langs).await
        }
        None => install_by_upload(gateway, peer, headers, person, query, body, langs).await,
    }
}

/// JSON 请求体的按来源安装：门槛通过后装来源指向的插件，回 201 和装好后的详情。
async fn install_by_source(
    gateway: Arc<Gateway>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    person: Person,
    request: InstallRequest,
    langs: Languages,
) -> Response {
    let finish = {
        let request = request.clone();
        let headers = headers.clone();
        let person = person.clone();
        move |gateway: Arc<Gateway>| {
            let request = request.clone();
            let headers = headers.clone();
            async move {
                let installed = async {
                    let id = gateway.plugins.install_source(&request).await?.id;
                    gateway
                        .plugins
                        .installed_plugin(&id, &languages(&headers))
                        .ok_or_else(|| InstallError::NotInstalled(id.to_string()))
                }
                .await;
                response_for_requester(
                    &gateway,
                    peer,
                    &headers,
                    &person,
                    installed,
                    |gateway, peer, person, plugin| {
                        super::bot_projection::installed_response(
                            gateway,
                            peer,
                            &headers,
                            person,
                            plugin,
                            StatusCode::CREATED,
                        )
                    },
                )
            }
        }
    };
    let gate =
        install_gates::install_source(&gateway, peer, &headers, &person, &request, finish).await;
    match install_gates::settle(gate, "install", &langs).await {
        Ok(response) | Err(response) => response,
    }
}

/// 上传插件包安装：门槛通过后装上传的包，回 201 和装好后的详情。
async fn install_by_upload(
    gateway: Arc<Gateway>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    person: Person,
    query: InstallQuery,
    body: Bytes,
    langs: Languages,
) -> Response {
    // 门槛的预览和闭包各用一份：闭包装的是主人批准后的同一份包。
    let package = match package(&langs, &headers, &body) {
        Ok(package) => package,
        Err(e) => return install_error(&e),
    };
    let for_install = package.clone();
    let selected_devices = comma_list(&query.devices);
    let changes = install_gate_checks::upload_changes_permissions(&query);
    let finish = {
        let headers = headers.clone();
        let person = person.clone();
        move |gateway: Arc<Gateway>| {
            let package = for_install.clone();
            let headers = headers.clone();
            let query = query.clone();
            async move {
                let installed = async {
                    let id = install_upload_package(&gateway, &headers, &query, package)
                        .await?
                        .id;
                    gateway
                        .plugins
                        .installed_plugin(&id, &languages(&headers))
                        .ok_or_else(|| InstallError::NotInstalled(id.to_string()))
                }
                .await;
                response_for_requester(
                    &gateway,
                    peer,
                    &headers,
                    &person,
                    installed,
                    |gateway, peer, person, plugin| {
                        super::bot_projection::installed_response(
                            gateway,
                            peer,
                            &headers,
                            person,
                            plugin,
                            StatusCode::CREATED,
                        )
                    },
                )
            }
        }
    };
    let gate = install_gates::install_upload(
        &gateway,
        peer,
        &headers,
        &person,
        (&package, &selected_devices, changes),
        finish,
    )
    .await;
    match install_gates::settle(gate, "install", &langs).await {
        Ok(response) | Err(response) => response,
    }
}

pub async fn retry(
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
        Err(error) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &error.to_string());
        }
    };
    gateway.note_languages(peer.map(|Extension(peer)| peer), &headers);
    match gateway.plugins.retry_failed_install(id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => install_error(&error),
    }
}

/// 上传的插件包，`?grant=` 是主人确认的权限。包已在门槛的预览里解析过，这里直接装。
async fn install_upload_package(
    gateway: &Arc<Gateway>,
    headers: &HeaderMap,
    query: &InstallQuery,
    package: PluginPackage,
) -> Result<PluginView, InstallError> {
    let langs = languages(headers);
    let granted = granted(query)?;
    let enabled = enabled(query)?;
    let secrets = secrets(headers)?;
    let given = settings(headers)?;
    gateway
        .plugins
        .store_input_values(&package.manifest, &secrets, None, &langs)?;
    let settings = crate::plugin::settings::install_values(&package, &given, &langs)?;
    gateway
        .plugins
        .install_detached_enabled(
            package,
            granted,
            enabled,
            query.lan_webui,
            &comma_list(&query.devices),
            settings,
            langs.clone(),
        )
        .await
}

pub async fn uninstall(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<UninstallQuery>,
    body: Bytes,
) -> Response {
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string()),
    };
    let langs = languages(&headers);
    gateway.note_languages(peer.map(|Extension(peer)| peer), &headers);
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    let request = if body.is_empty() {
        UninstallRequest::default()
    } else {
        match json_body::<UninstallRequest>(&headers, &body) {
            Some(Ok(request)) => request,
            Some(Err(error)) => return install_error(&error),
            None => return install_error(&invalid_body(&headers, "expected application/json")),
        }
    };
    // 批准连接重组只有主人能做（ADR-0088：这不是“装、删、升级应用”能换来的）。
    let approve_connections =
        permissions::require(&gateway, peer, &headers, PermissionItem::Owner).is_none();
    let finish = {
        let id = id.clone();
        let langs = langs.clone();
        move |gateway: Arc<Gateway>| {
            let id = id.clone();
            async move {
                match gateway
                    .plugins
                    .uninstall_detached(
                        id,
                        query.purge,
                        request.providers,
                        approve_connections,
                        langs,
                    )
                    .await
                {
                    Ok(report) => Json(report).into_response(),
                    Err(e) => install_error(&e),
                }
            }
        }
    };
    let gate = install_gates::uninstall(&gateway, peer, &headers, &person, &id, finish).await;
    match install_gates::settle(gate, "uninstall", &langs).await {
        Ok(response) => response,
        Err(response) => response,
    }
}
