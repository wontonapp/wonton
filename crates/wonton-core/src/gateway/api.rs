//! 插件管理 API（ADR-0017 的“接口：App 和核心”）。接口按权限项分（ADR-0088，[`super::permissions::require`]）：
//! 装卸、升级和可选权限开关要“装、删、升级应用”，读插件、看详情和每个人自己的接口只认已配对设备
//! （[`plugin_denied`] 认出设备后由处理器按人过滤）。响应是 `wonton-protocol` 的类型，文字按 `Accept-Language` 挑语言。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/plugins` | 已安装的插件（`InstalledPlugins`） |
//! | GET | `/_wonton/api/plugins/{id}` | 已安装插件的详情（`PluginDetail`） |
//! | POST | `/_wonton/api/plugins/preview` | 请求体是来源（`InstallSource`）或插件包，返回要主人确认的内容（`InstallPreview`）；上传的包暂存到确认，`source` 是 `upload:sha256:…` |
//! | POST | `/_wonton/api/plugins` | 按来源安装（`InstallRequest`），或者上传插件包加 `?grant=权限,权限`（可加 `&lan_webui=true`）；`grant` 含全部必需权限，可选项放在 `enabled_permissions`（201 `InstalledPlugin`） |
//! | POST | `/_wonton/api/plugins/{id}/start` | 启动已停止的插件；已运行时不重复启动，就绪并核对完成后返回（200 `InstalledPlugin`）。容器不在时按安装记录照安装的路径重建再启动（网络不在也重建），做不到按原因报类别：镜像拉不到 `image_pull_failed`，安装记录读不出 `record_unusable` |
//! | GET | `/_wonton/api/plugins/{id}/lifecycle` | 与启动接口相同的鉴权，返回持久化意图和恢复日志（200 `PluginLifecycle`），不把容器观测状态当作主人的意图 |
//! | POST | `/_wonton/api/plugins/{id}/stop` | 与启动接口相同的鉴权，持久化明确停止意图；幂等停止后返回（200 `InstalledPlugin`），忙时拒绝，不在后台重新启动 |
//! | PUT | `/_wonton/api/plugins/{id}/permissions/{permission}` | 开关一项可选权限（`PermissionUpdate`），重建完返回详情（200 `PluginDetail`） |
//! | PUT | `/_wonton/api/plugins/{id}/scope` | 主人提交 `PluginScope`；不支持范围设置时返回 501 `unsupported`，不保存 |
//! | GET、PUT | `/_wonton/api/bots/{id}/{bot}/plugin-access/{target}` | 插件 `{id}` 的 Bot `{bot}` 能不能用 `{target}` 的工具（`BotPluginAccess`）；主人 `PUT` 一个 `BotPluginAccessUpdate` 单独开关，总线按它过滤这个 Bot 的工具（见 [`super::bot_plugin_access`]）。Bot 不在 404 `not_found`，`{target}` 没装 `not_installed`，工具策略读不出 503 `policy_unavailable` |
//! | DELETE | `/_wonton/api/plugins/{id}?purge=true` | 卸载；`purge` 时删除数据 |
//! | POST | `/_wonton/api/plugins/{id}/upgrade/preview` | 请求体为空或 `{}` 时取能升到的版本，带 `source` 或者插件包时换到主人选的，可以比装着的旧；返回要主人确认的内容（`InstallPreview`） |
//! | POST | `/_wonton/api/plugins/{id}/upgrade` | 按预览升级（`UpgradeRequest`），或者上传插件包加 `?grant=权限,权限`；换完容器才返回（`UpgradeResult`） |
//! | GET | `/_wonton/api/images/{sha256}/{文件名}` | 插件的图标（PNG），地址由上面的响应给出 |
//!
//! 手动启动把意图设为运行并重置恢复额度。新安装默认要运行；旧记录没有意图时只观察。中断日志保留
//! `category`、`reason`、`stage`、`attempts` 和 Unix 秒 `next_retry_at`，进程中断的类别是 `interrupted`。
//! 恢复最多后台尝试五次，间隔 30、60、120、240、300 秒；核心重启不重置额度。空的重试时间表示执行中或已耗尽。
//!
//! 生命周期查询和停止沿用启动接口的 `InstallApps` 权限项：主人及获授此项的家人都能调用，
//! 未获授的家人不能调用；不另加仅限主人的策略，网页来源限制也与启动相同。
//!
//! 停止先保存 `stopped` 意图和 `stopping` 日志；Docker 停止失败或超时保留原因并显示降级，不安排自动启动。
//!
//! 商店的接口在 [`super::store_api`]，服务器设置和插件资源上限的接口在 [`super::settings_api`]，镜像加速在
//! [`super::registry_mirror`]，模型服务商在 [`super::model_providers`]，引导在 [`super::onboarding`]。
//!
//! 插件包是 compose 文本，或者 tar、tar.gz、zip 压缩包（按内容认）；JSON 请求体（`Content-Type: application/json`）按来源预览和安装。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::header::{ACCEPT_LANGUAGE, AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use hyper::{HeaderMap, StatusCode};
use serde::Deserialize;
use serde_json::json;
use wonton_protocol::{
    AddedTools, InstallRequest, InstallSource, PermissionItem, UninstallRequest, UpgradeRequest,
    UpgradeResult,
};

pub(super) use super::api_body::{MAX_UPLOAD_BYTES, invalid_body, json_body, package};
use super::api_switch::{enabled, granted, secrets, settings, upload_parts};
use super::{Gateway, Person, TunnelPeer, install_gates, isolation, pages, permissions};

pub use super::api_switch::set_plugin_permission;
use crate::l10n;
use crate::plugin::gallery::Variant;
use crate::plugin::image_worker::ImageError;
use crate::plugin::languages::Languages;
use crate::plugin::{InstallError, PluginId, PluginPackage, PluginView, UpgradeTo};

pub(super) fn languages(headers: &HeaderMap) -> Languages {
    Languages::from_header(headers.get(ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok()))
}

/// 带一个 `{error}` 参数的接口文字。
pub(super) fn text1(langs: &Languages, id: &'static str, error: &str) -> String {
    l10n::text(langs, id, &[("error", error.into())])
}

pub(crate) fn error_response(status: StatusCode, kind: &'static str, message: &str) -> Response {
    let body = Json(json!({"error": {"kind": kind, "message": message}}));
    let mut response = (status, body).into_response();
    response
        .headers_mut()
        .insert(pages::ERROR_HEADER, HeaderValue::from_static(kind));
    response
}

pub(crate) fn install_error(error: &InstallError) -> Response {
    if let InstallError::InUse { message, plugins } = error {
        let body = json!({"error": {"kind": "in_use", "message": message, "plugins": plugins}});
        let mut response = (StatusCode::CONFLICT, Json(body)).into_response();
        response
            .headers_mut()
            .insert(pages::ERROR_HEADER, HeaderValue::from_static("in_use"));
        return response;
    }
    if let InstallError::EgressProxyUnavailable { reason, message } = error {
        let body = json!({"error": {
            "kind": error.kind(),
            "message": message,
            "detail": reason,
        }});
        let mut response = (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response();
        response
            .headers_mut()
            .insert(pages::ERROR_HEADER, HeaderValue::from_static(error.kind()));
        return response;
    }
    let status = match error {
        InstallError::EgressProxyUnavailable { .. } => StatusCode::UNPROCESSABLE_ENTITY,
        InstallError::Invalid(_)
        | InstallError::UnsupportedRuntime(_)
        | InstallError::SettingsInvalid(..)
        | InstallError::SettingsSchemaInvalid(..) => StatusCode::BAD_REQUEST,
        InstallError::PermissionNotGranted(_) => StatusCode::FORBIDDEN,
        InstallError::Unsupported(_)
        | InstallError::GpuUnavailable(_)
        | InstallError::LanUnavailable(_) => StatusCode::UNPROCESSABLE_ENTITY,
        InstallError::Conflict(_)
        | InstallError::InUse { .. }
        | InstallError::NameTaken(_)
        | InstallError::Changed(_)
        | InstallError::PortTaken(_)
        | InstallError::RecordUnusable(_)
        | InstallError::CoreUpgrading(_) => StatusCode::CONFLICT,
        InstallError::NotInstalled(_) | InstallError::NotFound(_) => StatusCode::NOT_FOUND,
        InstallError::DockerUnavailable(_)
        | InstallError::OidcUnavailable(_)
        | InstallError::MatrixUnavailable(_)
        | InstallError::ModelsUnavailable(_)
        | InstallError::ImagesUnavailable(_)
        | InstallError::SourceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        InstallError::UpstreamUnavailable(_) => StatusCode::BAD_GATEWAY,
        InstallError::DockerTimedOut(_) => StatusCode::GATEWAY_TIMEOUT,
        InstallError::Docker(_) | InstallError::ImagePull(_) | InstallError::Unhealthy(_) => {
            StatusCode::BAD_GATEWAY
        }
        InstallError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    match error {
        InstallError::SettingsInvalid(message, fields) => settings_error(
            status,
            "settings_invalid",
            message,
            json!({"fields": fields}),
        ),
        InstallError::SettingsSchemaInvalid(message, problems) => settings_error(
            status,
            "settings_schema_invalid",
            message,
            json!({"problems": problems}),
        ),
        _ => error_response(status, error.kind(), &error.to_string()),
    }
}

/// 设置的两类校验失败：附加字段直接放在 `error` 对象上。
fn settings_error(
    status: StatusCode,
    kind: &'static str,
    message: &str,
    extra: serde_json::Value,
) -> Response {
    let mut error = json!({"kind": kind, "message": message});
    if let serde_json::Value::Object(extra) = extra
        && let serde_json::Value::Object(error) = &mut error
    {
        error.extend(extra);
    }
    let mut response = (status, Json(json!({"error": error}))).into_response();
    response
        .headers_mut()
        .insert(pages::ERROR_HEADER, HeaderValue::from_static(kind));
    response
}

/// 带没带主人令牌。
pub(crate) fn has_owner_token(gateway: &Gateway, headers: &HeaderMap) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|token| gateway.check_owner_token(token))
}

/// 只接受 `Authorization: Bearer <主人令牌>`：浏览器 Cookie 随请求自动带上，插件网页可以借此发起跨站请求。未授权时
/// 返回要直接回给客户端的响应。
pub(crate) fn denied(gateway: &Gateway, headers: &HeaderMap) -> Option<Response> {
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match token {
        Some(token) if gateway.check_owner_token(token) => None,
        _ => Some(error_response(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            &l10n::text(&languages(headers), "gateway-denied", &[]),
        )),
    }
}

/// 插件管理的读接口和每个人自己的接口：主人令牌，或者任何已配对设备（经隧道，且不是别的网页发起的）。
/// 设备是谁由调用方按人处理：家人的插件列表只列开放了的，设备列表只有自己的。
pub(crate) fn plugin_denied(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> Option<Response> {
    denied(gateway, headers)?;
    match gateway.device_person(peer.map(|Extension(peer)| peer)) {
        Some(_) => from_another_page(headers, &gateway.core_origin()),
        None => denied(gateway, headers),
    }
}

/// 设备的请求还要不是别的网页发起的（ADR-0017）：隧道身份随请求自动带上，App 的 WebView 里打开的插件网页
/// 发的请求也带着它，插件网页不能借它改管理设置。是别的网页发起时给拒绝。
pub(crate) fn from_another_page(headers: &HeaderMap, origin: &str) -> Option<Response> {
    if isolation::not_from_another_page(headers, origin) {
        return None;
    }
    Some(error_response(
        StatusCode::FORBIDDEN,
        "cross_site",
        &l10n::text(&languages(headers), "gateway-cross-site-api", &[]),
    ))
}

/// 过了 [`plugin_denied`] 的请求是谁发的，写审计用：主人令牌是 `owner_token`，设备是 `<用户名>/device:<id>`。
pub(super) fn acted_by(
    gateway: &Gateway,
    peer: Option<&Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> String {
    if denied(gateway, headers).is_none() {
        return "owner_token".to_owned();
    }
    if let Some(Extension(TunnelPeer(device))) = peer.copied()
        && let Some(person) = gateway.device_person(Some(TunnelPeer(device)))
    {
        return format!("{}/device:{device}", person.username);
    }
    "owner_token".to_owned()
}

/// 这些接口不接受会话 Cookie：请求的人要么带着主人令牌（主人），要么是经隧道的已配对设备（设备记录里的人）。
/// [`plugin_denied`] 已经把两样都没有的请求挡掉了。鉴权和取人之间设备记录可能消失（家人刚被移除），
/// 这时按认不出处理，回 [`plugin_denied`] 同样的拒绝，不编一个人出来。
pub(crate) fn requester(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> Result<Person, Box<Response>> {
    if denied(gateway, headers).is_none() {
        gateway.note_languages(None, headers);
        return Ok(Person::owner(gateway.owner.username()));
    }
    let person = gateway
        .device_person(peer.map(|Extension(peer)| peer))
        .ok_or_else(|| {
            Box::new(error_response(
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                &l10n::text(&languages(headers), "gateway-denied", &[]),
            ))
        })?;
    gateway.note_languages(peer.map(|Extension(peer)| peer), headers);
    Ok(person)
}

pub async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    gateway.note_languages(peer.map(|Extension(peer)| peer), &headers);
    let mut plugins = gateway
        .plugins
        .installed_checked(&languages(&headers))
        .await;
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    let open_apps = gateway.permissions(&person).open_apps;
    plugins
        .plugins
        .retain(|plugin| open_apps.allows_plugin(&plugin.id));
    Json(plugins).into_response()
}

pub async fn detail(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string()),
    };
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    let permissions = gateway.permissions(&person);
    if !permissions.open_apps.allows_plugin(id.as_str()) {
        return error_response(
            StatusCode::FORBIDDEN,
            "not_shared",
            &format!("应用 {id} 没有开放给家人"),
        );
    }
    match gateway
        .plugins
        .judged_detail(&id, &languages(&headers))
        .await
    {
        Some(mut detail) => {
            if permissions.allows(PermissionItem::ShareApps)
                && let Some(sharing) = gateway.sharing.as_ref()
            {
                let offer = gateway.plugins.share_offer(&id);
                if offer.installed && offer.has_webui {
                    detail.shares = match sharing
                        .shares
                        .plugin_shares(&offer, super::sharing::now_ms())
                    {
                        Ok(shares) => Some(shares),
                        Err(error) => return super::sharing::api::share_error(&error),
                    };
                }
            }
            install_gates::project_detail(&gateway, &person, id.as_str(), &mut detail);
            Json(detail).into_response()
        }
        None => install_error(&InstallError::NotInstalled(id.to_string())),
    }
}

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

#[derive(Clone, Deserialize)]
pub struct InstallQuery {
    #[serde(default)]
    pub(super) grant: String,
    /// 主人打开了开关 `lan_webui`。
    #[serde(default)]
    pub(super) lan_webui: bool,
    /// 主人打开了的可选项，逗号分隔（ADR-0057）。
    #[serde(default)]
    pub(super) enabled_permissions: String,
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
            let finish = {
                let request = request.clone();
                let headers = headers.clone();
                move |gateway: Arc<Gateway>| {
                    let request = request.clone();
                    let headers = headers.clone();
                    async move {
                        let installed = gateway.plugins.install_source(&request).await;
                        let id = match installed {
                            Ok(view) => view.id,
                            Err(e) => return install_error(&e),
                        };
                        match gateway.plugins.installed_plugin(&id, &languages(&headers)) {
                            Some(installed) => {
                                (StatusCode::CREATED, Json(installed)).into_response()
                            }
                            // 装好之后立刻被卸掉了。
                            None => install_error(&InstallError::NotInstalled(id.to_string())),
                        }
                    }
                }
            };
            let gate =
                install_gates::install_source(&gateway, peer, &headers, &person, &request, finish)
                    .await;
            match install_gates::settle(gate, "install", &langs).await {
                Ok(response) => response,
                Err(response) => response,
            }
        }
        None => {
            // 门槛的预览和闭包各用一份：闭包装的是主人批准后的同一份包。
            let package = match package(&langs, &headers, &body) {
                Ok(package) => package,
                Err(e) => return install_error(&e),
            };
            let for_install = package.clone();
            let finish = {
                let headers = headers.clone();
                move |gateway: Arc<Gateway>| {
                    let package = for_install.clone();
                    let headers = headers.clone();
                    let query = query.clone();
                    async move {
                        match install_upload_package(&gateway, &headers, &query, package).await {
                            Ok(view) => {
                                let id = view.id;
                                match gateway.plugins.installed_plugin(&id, &languages(&headers)) {
                                    Some(installed) => {
                                        (StatusCode::CREATED, Json(installed)).into_response()
                                    }
                                    None => {
                                        install_error(&InstallError::NotInstalled(id.to_string()))
                                    }
                                }
                            }
                            Err(e) => install_error(&e),
                        }
                    }
                }
            };
            let gate =
                install_gates::install_upload(&gateway, peer, &headers, &person, &package, finish)
                    .await;
            match install_gates::settle(gate, "install", &langs).await {
                Ok(response) => response,
                Err(response) => response,
            }
        }
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
            settings,
            langs.clone(),
        )
        .await
}

/// 升级预览的 JSON 请求体：`{}` 是升到提示的版本，带 `source` 是换到这个来源。
#[derive(Deserialize)]
struct UpgradeFrom {
    #[serde(default)]
    source: Option<String>,
}

pub async fn preview_upgrade(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::InstallApps)
    {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let to = match json_body::<UpgradeFrom>(&headers, &body) {
        Some(Err(e)) => return install_error(&e),
        Some(Ok(UpgradeFrom {
            source: Some(source),
        })) => UpgradeTo::Source(source),
        Some(Ok(UpgradeFrom { source: None })) => UpgradeTo::Offered,
        None if body.is_empty() => UpgradeTo::Offered,
        None => match package(&languages(&headers), &headers, &body) {
            Ok(package) => UpgradeTo::Upload(Box::new(package)),
            Err(e) => return install_error(&e),
        },
    };
    let langs = languages(&headers);
    match gateway.plugins.preview_upgrade(&id, to, &langs).await {
        Ok(mut preview) => {
            if let Ok(person) = requester(&gateway, peer, &headers) {
                install_gates::project_preview(&gateway, &person, &mut preview);
            }
            Json(preview).into_response()
        }
        Err(e) => install_error(&e),
    }
}

pub async fn upgrade(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<InstallQuery>,
    body: Bytes,
) -> Response {
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let langs = languages(&headers);
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    let finish = {
        let headers = headers.clone();
        let query = query.clone();
        let body = body.clone();
        let id = id.clone();
        move |gateway: Arc<Gateway>| {
            let headers = headers.clone();
            let query = query.clone();
            let id = id.clone();
            async move {
                let langs = languages(&headers);
                let before = match &gateway.mcp {
                    Some(mcp) => mcp.bus.tool_names(&id).await,
                    None => Err(l10n::text(&langs, "gateway-mcp-disabled", &[])),
                };
                let upgraded = match json_body::<UpgradeRequest>(&headers, &body) {
                    Some(Err(e)) => return install_error(&e),
                    Some(Ok(request)) => {
                        gateway.plugins.upgrade_source(&id, &request, &langs).await
                    }
                    None => match upload_parts(&languages(&headers), &headers, &body, &query)
                        .and_then(|parts| Ok((parts, settings(&headers)?)))
                    {
                        Ok(((grant, enabled, secrets, package), given)) => {
                            let plugins = &gateway.plugins;
                            let filled = plugins.store().secrets().load(&id).ok().flatten();
                            if let Err(e) = plugins.store_input_values(
                                &package.manifest,
                                &secrets,
                                filled.as_ref(),
                                &langs,
                            ) {
                                return install_error(&e);
                            }
                            plugins
                                .upgrade_upload(
                                    &id,
                                    package,
                                    grant,
                                    enabled,
                                    query.lan_webui,
                                    &given,
                                    &langs,
                                )
                                .await
                        }
                        Err(e) => return install_error(&e),
                    },
                };
                if let Err(e) = upgraded {
                    return install_error(&e);
                }
                // 主人经文件或来源升级后，插件不再归 Agent（ADR-0033）：归属和“每次都问”一起清掉。
                if let Err(e) = gateway.plugins.set_writer(&id, None) {
                    tracing::error!(plugin = %id, error = %e, "清插件的归属失败");
                }
                let added_tools = match &gateway.mcp {
                    Some(mcp) => mcp.bus.added_tools(&id, before).await,
                    None => AddedTools::NotCompared {
                        reason: l10n::text(&langs, "gateway-mcp-disabled", &[]),
                    },
                };
                match gateway.plugins.installed_plugin(&id, &langs) {
                    Some(plugin) => Json(UpgradeResult {
                        plugin,
                        added_tools,
                    })
                    .into_response(),
                    None => install_error(&InstallError::NotInstalled(id.to_string())),
                }
            }
        }
    };
    // 门槛要升级预览的内容写卡片，先把请求体读出来。
    let to = match json_body::<UpgradeFrom>(&headers, &body) {
        Some(Err(e)) => return install_error(&e),
        Some(Ok(UpgradeFrom {
            source: Some(source),
        })) => UpgradeTo::Source(source),
        Some(Ok(UpgradeFrom { source: None })) => UpgradeTo::Offered,
        None if body.is_empty() => UpgradeTo::Offered,
        None => match package(&languages(&headers), &headers, &body) {
            Ok(package) => UpgradeTo::Upload(Box::new(package)),
            Err(e) => return install_error(&e),
        },
    };
    let gate = install_gates::upgrade(&gateway, peer, &headers, &person, &id, to, finish).await;
    match install_gates::settle(gate, "upgrade", &langs).await {
        Ok(response) => response,
        Err(response) => response,
    }
}

#[derive(Deserialize)]

pub struct UninstallQuery {
    #[serde(default)]
    purge: bool,
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

/// 插件的图标。地址里有原文件的 sha256，同一个地址的内容不变，所以可以一直缓存。
pub async fn image(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((sha256, file)): Path<(String, String)>,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    let icon = l10n::text(&langs, "gateway-icon-not-found", &[]);
    let not_found = || error_response(StatusCode::NOT_FOUND, "not_found", &icon);
    let Some(variant) = Variant::parse(&file) else {
        return not_found();
    };
    match gateway.plugins.icon_png(&sha256, variant).await {
        Ok(Some(png)) => (
            [
                (CONTENT_TYPE, "image/png"),
                (CACHE_CONTROL, "private, max-age=31536000, immutable"),
            ],
            png,
        )
            .into_response(),
        Ok(None) => not_found(),
        Err(ImageError::Refused(reason) | ImageError::TimedOut(reason)) => error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_input",
            &text1(&langs, "gateway-icon-failed", reason.as_str()),
        ),
        Err(e @ ImageError::Unavailable(_)) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "images_unavailable",
            &e.to_string(),
        ),
    }
}
