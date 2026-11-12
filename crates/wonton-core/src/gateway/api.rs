//! 插件管理 API（ADR-0017 的“接口：App 和核心”）。接受主人令牌和经隧道的已配对设备（不能是网页发起的，见
//! [`plugin_denied`]）；局域网设置和设备管理只认主人令牌。响应是 `wonton-protocol` 的类型，文字按 `Accept-Language`
//! 挑语言。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/plugins` | 已安装的插件（`InstalledPlugins`） |
//! | GET | `/_wonton/api/plugins/{id}` | 已安装插件的详情（`PluginDetail`） |
//! | POST | `/_wonton/api/plugins/preview` | 请求体是来源（`InstallSource`）或插件包，返回要主人确认的内容（`InstallPreview`）；上传的包暂存到确认，`source` 是 `upload:sha256:…` |
//! | POST | `/_wonton/api/plugins` | 按来源安装（`InstallRequest`），或者上传插件包加 `?grant=权限,权限`（可加 `&lan_webui=true`）；权限必须与清单声明的一致（201 `InstalledPlugin`） |
//! | POST | `/_wonton/api/plugins/{id}/start` | 启动已停止的插件；已运行时不重复启动，就绪并核对完成后返回（200 `InstalledPlugin`） |
//! | DELETE | `/_wonton/api/plugins/{id}?purge=true` | 卸载；`purge` 时删除数据 |
//! | POST | `/_wonton/api/plugins/{id}/upgrade/preview` | 请求体为空或 `{}` 时取能升到的版本，带 `source` 或者插件包时换到主人选的，可以比装着的旧；返回要主人确认的内容（`InstallPreview`） |
//! | POST | `/_wonton/api/plugins/{id}/upgrade` | 按预览升级（`UpgradeRequest`），或者上传插件包加 `?grant=权限,权限`；换完容器才返回（`UpgradeResult`） |
//! | PUT | `/_wonton/api/plugins/{id}/limits` | 请求体是 `PluginLimits`，改资源上限后返回 `PluginDetail` |
//! | PUT | `/_wonton/api/plugins/{id}/lan-webui` | 请求体是 `LanWebuiUpdate`，改网页的局域网端口开关，重建完返回 `PluginDetail` |
//! | DELETE | `/_wonton/api/plugins/{id}/save-grants?folder=` | 撤销 Bot 存文件的一条授权（ADR-0027），返回 `PluginDetail` |
//! | GET/PUT | `/_wonton/api/egress-proxy` | 出网代理的设置（`EgressProxySettings` / `EgressProxyUpdate`），改完返回 |
//! | GET/PUT | `/_wonton/api/registry-mirror` | 镜像加速的设置（`RegistryMirrorSettings` / `RegistryMirrorUpdate`） |
//! | POST | `/_wonton/api/registry-mirror/test` | 按摘要测速给定地址和 Docker Hub（`RegistryMirrorTests`） |
//! | GET/PUT | `/_wonton/api/onboarding` | 引导状态和官方推荐清单（`Onboarding`） |
//! | GET | `/_wonton/api/store?refresh=true` | 商店（`StoreListing`）；`refresh` 时官方索引、社区插件和已安装插件的新版本都不等缓存过期 |
//! | GET | `/_wonton/api/store/plugin?source=github:…` | 来源最新的版本的详情（`PluginDetail`） |
//! | POST | `/_wonton/api/store/sources` | 添加来源（`AddSourceRequest`），能读到才写下来（201 `AddedSource`） |
//! | DELETE | `/_wonton/api/store/sources?address=…` | 删除添加的来源，已安装的插件不动 |
//! | GET | `/_wonton/api/images/{sha256}/{文件名}` | 插件的图标（PNG），地址由上面的响应给出 |
//! | GET | `/_wonton/api/storage` | 存数据的那块盘的总量和可用量（`StorageUsage`） |
//! | GET | `/_wonton/api/lan` | 局域网设置（`LanSettings`），当场检测一次（ADR-0019） |
//! | PUT | `/_wonton/api/lan` | 请求体是 `LanChoice`，改选端口开在哪些地址上，重建完返回新的 `LanSettings` |
//! | GET | `/_wonton/api/models/providers` | 模型服务商列表（`ModelProviders`） |
//! | PUT/POST | `/_wonton/api/models/providers/{id}[/test]` | 保存或只测试模型服务商 key |
//! | PUT/POST | `/_wonton/api/models/custom-provider[/test]` | 保存或只测试自定义服务商 |
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
use serde::de::DeserializeOwned;
use serde_json::json;
use wonton_protocol::{
    AddSourceRequest, AddedTools, AskEveryUpdate, EgressProxyUpdate, InstallRequest, InstallSource,
    LanChoice, LanWebuiUpdate, PluginLimits, UpgradeRequest, UpgradeResult,
};

use super::{Gateway, TunnelPeer, isolation, pages};
use crate::plugin::gallery::Variant;
use crate::plugin::image_worker::ImageError;
use crate::plugin::languages::Languages;
use crate::plugin::package_archive;
use crate::plugin::{InstallError, Permission, PluginId, PluginPackage, PluginView, UpgradeTo};

/// 上传插件包的请求体上限：压缩包里可以带着插件包以外的文件。插件包本身的大小由解析时检查。
pub const MAX_UPLOAD_BYTES: usize = package_archive::MAX_ARCHIVE_BYTES;

pub(super) fn languages(headers: &HeaderMap) -> Languages {
    Languages::from_header(headers.get(ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok()))
}

/// JSON 请求体是按来源预览或安装（`InstallSource`、`InstallRequest`）；其余是上传的插件包。解析不了时是
/// `InstallError::Invalid`。
fn json_body<T: DeserializeOwned>(
    headers: &HeaderMap,
    body: &[u8],
) -> Option<Result<T, InstallError>> {
    let json = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("application/json"));
    json.then(|| {
        serde_json::from_slice(body).map_err(|e| InstallError::Invalid(format!("请求体不对：{e}")))
    })
}

/// 请求体里的插件包：压缩包（按内容认格式，见 [`package_archive`]），或者 compose 文本。
fn package(headers: &HeaderMap, body: &[u8]) -> Result<PluginPackage, InstallError> {
    let tar = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("application/x-tar"));
    if tar || package_archive::is_archive(body) {
        return Ok(PluginPackage::from_archive(body)?);
    }
    let source = std::str::from_utf8(body).map_err(|_| {
        InstallError::Invalid("不是插件包：要 tar、tar.gz、zip 或 compose 文件".to_owned())
    })?;
    Ok(PluginPackage::parse(source)?)
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
        InstallError::Invalid(_) => StatusCode::BAD_REQUEST,
        InstallError::PermissionNotGranted(_) => StatusCode::FORBIDDEN,
        InstallError::Unsupported(_)
        | InstallError::GpuUnavailable(_)
        | InstallError::LanUnavailable(_) => StatusCode::UNPROCESSABLE_ENTITY,
        InstallError::Conflict(_)
        | InstallError::NameTaken(_)
        | InstallError::Changed(_)
        | InstallError::OwnerProfileIncomplete
        | InstallError::CoreUpgrading => StatusCode::CONFLICT,
        InstallError::NotInstalled(_) | InstallError::NotFound(_) => StatusCode::NOT_FOUND,
        InstallError::DockerUnavailable(_)
        | InstallError::OidcUnavailable(_)
        | InstallError::MatrixUnavailable(_)
        | InstallError::ModelsUnavailable(_)
        | InstallError::ImagesUnavailable(_)
        | InstallError::SourceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        InstallError::UpstreamUnavailable(_) => StatusCode::BAD_GATEWAY,
        InstallError::Docker(_) | InstallError::ImagePull(_) | InstallError::Unhealthy(_) => {
            StatusCode::BAD_GATEWAY
        }
        InstallError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error_response(status, error.kind(), &error.to_string())
}

/// 只接受 `Authorization: Bearer <主人令牌>`：浏览器 Cookie 随请求自动带上，插件网页可以借此发起跨站请求。未授权时
/// 返回要直接回给客户端的响应。
/// 过了 [`plugin_denied`] 的请求是谁发的，写审计用。
pub(super) fn acted_by(
    gateway: &Gateway,
    peer: Option<&Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> String {
    match (denied(gateway, headers), peer) {
        (Some(_), Some(Extension(TunnelPeer(device)))) => format!("device:{device}"),
        _ => "owner_token".to_owned(),
    }
}

pub(super) fn denied(gateway: &Gateway, headers: &HeaderMap) -> Option<Response> {
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match token {
        Some(token) if gateway.check_owner_token(token) => None,
        _ => Some(error_response(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "这台设备没有配对或者已被移除，要重新配对；命令行要带 Authorization: Bearer <令牌>",
        )),
    }
}

/// 插件管理接口的鉴权：主人令牌，或者经隧道的已配对设备。设备身份随隧道上的每个请求自动带上，App 的 WebView 里打开的
/// 插件网页发的请求也带着它，所以设备的请求还要不是别的网页发起的，读也一样（ADR-0017）：插件网页不能拿商店刷新消耗
/// 限速，也不能探测装了哪些插件。
pub(crate) fn plugin_denied(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> Option<Response> {
    let denied = denied(gateway, headers)?;
    if gateway
        .device_user(peer.map(|Extension(peer)| peer))
        .is_none()
    {
        return Some(denied);
    }
    if isolation::not_from_another_page(headers, &gateway.core_origin()) {
        return None;
    }
    Some(error_response(
        StatusCode::FORBIDDEN,
        "cross_site",
        "插件管理接口不接受其他网页发来的请求",
    ))
}

pub async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let accept = headers.get(ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok());
    gateway.plugins.note_owner_languages(accept);
    Json(
        gateway
            .plugins
            .installed_checked(&languages(&headers))
            .await,
    )
    .into_response()
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
    match gateway
        .plugins
        .judged_detail(&id, &languages(&headers))
        .await
    {
        Some(detail) => Json(detail).into_response(),
        None => install_error(&InstallError::NotInstalled(id.to_string())),
    }
}

pub async fn preview(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
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
        None => match package(&headers, &body) {
            Ok(package) => gateway.plugins.preview_upload(package, &langs).await,
            Err(e) => return install_error(&e),
        },
    };
    match preview {
        Ok(preview) => Json(preview).into_response(),
        Err(e) => install_error(&e),
    }
}

#[derive(Deserialize)]
pub struct InstallQuery {
    #[serde(default)]
    grant: String,
    /// 主人打开了开关 `lan_webui`。
    #[serde(default)]
    lan_webui: bool,
}

pub async fn install(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Query(query): Query<InstallQuery>,
    body: Bytes,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let installed = match json_body::<InstallRequest>(&headers, &body) {
        Some(Err(e)) => return install_error(&e),
        Some(Ok(request)) => gateway.plugins.install_source(&request).await,
        None => install_upload(&gateway, &headers, &query, &body).await,
    };
    let id = match installed {
        Ok(view) => view.id,
        Err(e) => return install_error(&e),
    };
    match gateway.plugins.installed_plugin(&id, &languages(&headers)) {
        Some(installed) => (StatusCode::CREATED, Json(installed)).into_response(),
        // 装好之后立刻被卸掉了。
        None => install_error(&InstallError::NotInstalled(id.to_string())),
    }
}

/// 上传的插件包，`?grant=` 是主人确认的权限。
async fn install_upload(
    gateway: &Arc<Gateway>,
    headers: &HeaderMap,
    query: &InstallQuery,
    body: &[u8],
) -> Result<PluginView, InstallError> {
    let granted = granted(query)?;
    let package = package(headers, body)?;
    gateway
        .plugins
        .install_detached(package, granted, query.lan_webui)
        .await
}

fn granted(query: &InstallQuery) -> Result<Vec<Permission>, InstallError> {
    query
        .grant
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::parse)
        .collect::<Result<Vec<Permission>, _>>()
        .map_err(|e: <Permission as std::str::FromStr>::Err| InstallError::Invalid(e.to_string()))
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
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
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
        None => match package(&headers, &body) {
            Ok(package) => UpgradeTo::Upload(Box::new(package)),
            Err(e) => return install_error(&e),
        },
    };
    let langs = languages(&headers);
    match gateway.plugins.preview_upgrade(&id, to, &langs).await {
        Ok(preview) => Json(preview).into_response(),
        Err(e) => install_error(&e),
    }
}

const NO_BUS: &str = "核心没有启用 MCP 总线";

pub async fn upgrade(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<InstallQuery>,
    body: Bytes,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let before = match &gateway.mcp {
        Some(mcp) => mcp.bus.tool_names(&id).await,
        None => Err(NO_BUS.to_owned()),
    };
    let upgraded = match json_body::<UpgradeRequest>(&headers, &body) {
        Some(Err(e)) => return install_error(&e),
        Some(Ok(request)) => gateway.plugins.upgrade_source(&id, &request).await,
        None => match granted(&query).and_then(|grant| Ok((grant, package(&headers, &body)?))) {
            Ok((grant, package)) => {
                let plugins = &gateway.plugins;
                plugins
                    .upgrade_upload(&id, package, grant, query.lan_webui)
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
            reason: NO_BUS.to_owned(),
        },
    };
    match gateway.plugins.installed_plugin(&id, &languages(&headers)) {
        Some(plugin) => Json(UpgradeResult {
            plugin,
            added_tools,
        })
        .into_response(),
        None => install_error(&InstallError::NotInstalled(id.to_string())),
    }
}

#[derive(Deserialize)]
pub struct StoreQuery {
    #[serde(default)]
    refresh: bool,
}

pub async fn store(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Query(query): Query<StoreQuery>,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
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
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Query(query): Query<StorePluginQuery>,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    match gateway
        .plugins
        .store_plugin(&query.source, &languages(&headers))
        .await
    {
        Ok(detail) => Json(detail).into_response(),
        Err(e) => install_error(&e),
    }
}

pub async fn add_store_source(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let request = match serde_json::from_slice::<AddSourceRequest>(&body) {
        Ok(request) => request,
        Err(e) => return install_error(&InstallError::Invalid(format!("请求体不对：{e}"))),
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
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Query(query): Query<RemoveSourceQuery>,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    match gateway.plugins.remove_store_source(&query.address) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => install_error(&e),
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
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string()),
    };
    match gateway.plugins.uninstall_detached(id, query.purge).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => install_error(&e),
    }
}

pub async fn storage(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    match gateway.plugins.storage_usage() {
        Ok(usage) => Json(usage).into_response(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_unavailable",
            &format!("查磁盘空间失败：{e}"),
        ),
    }
}

pub async fn lan(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    match gateway.plugins.lan_settings(&languages(&headers)).await {
        Ok(settings) => Json(settings).into_response(),
        Err(e) => install_error(&e),
    }
}

pub async fn set_lan(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    let choice = match serde_json::from_slice::<LanChoice>(&body) {
        Ok(choice) => choice,
        Err(e) => {
            let message = format!("请求体不是局域网设置的选择：{e}");
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &message);
        }
    };
    match gateway
        .plugins
        .set_lan_choice(choice, languages(&headers))
        .await
    {
        Ok(settings) => Json(settings).into_response(),
        Err(e) => install_error(&e),
    }
}

/// 出网代理的设置（ADR-0023）：先返回最近一次探测的结果。
pub async fn egress_proxy(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    match gateway.plugins.egress_settings(&languages(&headers)) {
        Ok(settings) => Json(settings).into_response(),
        Err(e) => install_error(&e),
    }
}

/// 改出网代理。探测过、存下、需要时重启完插件才返回。
pub async fn set_egress_proxy(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let update = match serde_json::from_slice::<EgressProxyUpdate>(&body) {
        Ok(update) => update,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string());
        }
    };
    let langs = languages(&headers);
    match gateway.plugins.set_egress_proxy(update, langs).await {
        Ok(settings) => Json(settings).into_response(),
        Err(e) => install_error(&e),
    }
}

/// 改网页的局域网端口开关（ADR-0023），重建完返回详情。
pub async fn set_lan_webui(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let update = match serde_json::from_slice::<LanWebuiUpdate>(&body) {
        Ok(update) => update,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string());
        }
    };
    let langs = languages(&headers);
    match gateway
        .plugins
        .set_lan_webui(id, update.enabled, langs)
        .await
    {
        Ok(detail) => Json(detail).into_response(),
        Err(e) => install_error(&e),
    }
}

/// 改插件每个服务的资源上限（ADR-0037），请求断开时更新仍继续。
pub async fn set_limits(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let limits = match serde_json::from_slice::<PluginLimits>(&body) {
        Ok(limits) => limits,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string()),
    };
    match gateway
        .plugins
        .set_limits(id, limits, languages(&headers))
        .await
    {
        Ok(detail) => Json(detail).into_response(),
        Err(e) => install_error(&e),
    }
}

/// 开关“每次都问”（ADR-0033）：Agent 写的插件每次更新都发卡片。改完返回详情。
pub async fn set_ask_every_update(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let update = match serde_json::from_slice::<AskEveryUpdate>(&body) {
        Ok(update) => update,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string());
        }
    };
    let langs = languages(&headers);
    match gateway
        .plugins
        .set_ask_every_update(&id, update.enabled, &langs)
    {
        Ok(detail) => Json(detail).into_response(),
        Err(e) => install_error(&e),
    }
}

#[derive(Deserialize)]
pub struct SaveGrantQuery {
    folder: String,
}

/// 撤销 Bot 存文件的一条授权（ADR-0027），返回详情。
pub async fn revoke_save_grant(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<SaveGrantQuery>,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let Some(mcp) = &gateway.mcp else {
        let message = "核心没有启用 MCP 总线";
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "mcp_unavailable", message);
    };
    let langs = languages(&headers);
    if gateway.plugins.installed_detail(&id, &langs).is_none() {
        return install_error(&InstallError::NotInstalled(id.to_string()));
    }
    if let Err(e) = mcp.bus.revoke_save_grant(&id, &query.folder) {
        return install_error(&e.into());
    }
    match gateway.plugins.judged_detail(&id, &langs).await {
        Some(detail) => Json(detail).into_response(),
        None => install_error(&InstallError::NotInstalled(id.to_string())),
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
    let not_found = || error_response(StatusCode::NOT_FOUND, "not_found", "没有这张图片");
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
        Err(ImageError::Refused(reason)) => error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_input",
            &format!("图标处理不了：{reason}"),
        ),
        Err(e @ ImageError::Unavailable(_)) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "images_unavailable",
            &e.to_string(),
        ),
    }
}
