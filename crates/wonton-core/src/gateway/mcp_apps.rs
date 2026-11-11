//! MCP Apps 的 HTTP 入口（ADR-0052 的“Bot 往面板上放页面”）：宿主页、工具调用、资源读取和发消息。
//!
//! 宿主页由核心给，App 的 WebView 直接加载；界面放在它里面带 `sandbox` 的 iframe 里，只能经
//! postMessage 和宿主页说 MCP Apps 的话。工具调用只限提供这个界面的插件声明给界面用的工具，
//! 经总线转发，敏感的照常请主人确认。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use rmcp::model::ResourceContents;
use serde::Deserialize;
use serde_json::{Value, json};
use wonton_protocol::{PanelScope, TabRef};

use super::api::error_response;
use super::{Gateway, TunnelPeer, isolation};
use crate::l10n;
use crate::plugin::PluginId;
use crate::plugin::pins::PanelPin;

const HOST_PAGE: &str = include_str!("mcp_apps/host.html");

/// 同时留着的界面调用结果。
const MAX_CALLS: usize = 64;
/// 结果留这么久，页面没来取也清掉。
const KEEP_CALL: Duration = Duration::from_secs(10 * 60);
/// 界面资源最多这么大：面板不是拿它传大文件的。
pub(super) const MAX_RESOURCE: usize = 8 << 20;

struct Call {
    done: bool,
    result: Option<Value>,
    error: Option<(String, String)>,
    created: Instant,
}

/// 界面发起的工具调用：先返回标识，页面轮询结果。确认可能要等主人，所以不在请求里等。
#[derive(Default)]
pub(crate) struct AppCalls {
    calls: Mutex<HashMap<String, Call>>,
    /// 界面发消息用的主人会话：核心跑着就一直复用这一台设备，不每条消息登进登出刷设备列表。
    owner_session: tokio::sync::Mutex<Option<crate::matrix::Login>>,
}

/// 界面替主人发消息用的设备名。
const APP_SENDER_DEVICE: &str = "Wonton 核心（界面发消息）";

impl AppCalls {
    fn start(&self, id: &str) {
        let mut calls = self.calls.lock().expect("界面调用表锁中毒");
        calls.retain(|_, call| call.created.elapsed() < KEEP_CALL);
        if calls.len() >= MAX_CALLS
            && let Some(oldest) = calls
                .iter()
                .min_by_key(|(_, call)| call.created)
                .map(|(id, _)| id.clone())
        {
            calls.remove(&oldest);
        }
        calls.insert(
            id.to_owned(),
            Call {
                done: false,
                result: None,
                error: None,
                created: Instant::now(),
            },
        );
    }

    fn finish(&self, id: &str, result: Value) {
        if let Some(call) = self.calls.lock().expect("界面调用表锁中毒").get_mut(id) {
            call.done = true;
            call.result = Some(result);
        }
    }

    fn fail(&self, id: &str, kind: String, message: String) {
        if let Some(call) = self.calls.lock().expect("界面调用表锁中毒").get_mut(id) {
            call.done = true;
            call.error = Some((kind, message));
        }
    }

    /// 复用的主人会话；服务器拒了令牌（设备被删）就清掉，下次重登。
    async fn owner_login(
        &self,
        matrix: &crate::matrix::Matrix,
        owner: &str,
    ) -> Result<crate::matrix::Login, crate::matrix::MatrixError> {
        let mut guard = self.owner_session.lock().await;
        if let Some(login) = guard.as_ref() {
            return Ok(login.clone());
        }
        let login = matrix.login(owner, APP_SENDER_DEVICE).await?;
        *guard = Some(login.clone());
        Ok(login)
    }

    async fn drop_owner_session(&self, stale: &crate::matrix::Login) {
        let mut guard = self.owner_session.lock().await;
        if guard.as_ref() == Some(stale) {
            *guard = None;
        }
    }

    /// 取一次结果：没完成时返回 `None`，完成时连着调用一起删掉。
    fn take(&self, id: &str) -> Option<Value> {
        let mut calls = self.calls.lock().expect("界面调用表锁中毒");
        if !calls.get(id)?.done {
            return None;
        }
        let call = calls.remove(id)?;
        Some(match call.error {
            Some((kind, message)) => {
                json!({"done": true, "error": {"kind": kind, "message": message}})
            }
            None => json!({"done": true, "result": call.result}),
        })
    }
}

fn unauthorized(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> Option<Response> {
    if gateway
        .device_user(peer.map(|Extension(peer)| peer))
        .is_none()
        && !super::api::has_owner_token(gateway, headers)
    {
        return Some(error_response(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "这台设备没有配对或者已被移除，要重新配对",
        ));
    }
    if !isolation::not_from_another_page(headers, &gateway.core_origin()) {
        return Some(error_response(
            StatusCode::FORBIDDEN,
            "cross_site",
            "界面接口不接受其他网页发来的请求",
        ));
    }
    None
}

fn failed(gateway: &Gateway, status: StatusCode, error: impl std::fmt::Display) -> Response {
    let langs = gateway.plugins.owner_languages();
    error_response(
        status,
        "source_unavailable",
        &l10n::text(
            &langs,
            "mcp-app-failed",
            &[("error", error.to_string().into())],
        ),
    )
}

#[allow(clippy::result_large_err)]
fn find_pin(
    gateway: &Gateway,
    pin: &str,
) -> Result<Option<(PluginId, String, PanelPin)>, Response> {
    gateway
        .plugins
        .store()
        .pins()
        .find(pin)
        .map_err(|e| failed(gateway, StatusCode::INTERNAL_SERVER_ERROR, e))
}

#[derive(Deserialize)]
pub(super) struct PinRef {
    pin: String,
}

#[allow(clippy::result_large_err)]
fn resource_pin(
    gateway: &Gateway,
    pin: &str,
) -> Result<(PluginId, PluginId, String, PanelPin, String), Response> {
    let Some((owner, bot, found)) = find_pin(gateway, pin)? else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "没有这个钉页",
        ));
    };
    let wonton_protocol::TabRef::UiResource { resource_uri, .. } = &found.tab else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "这一页不是 MCP Apps 界面",
        ));
    };
    let TabRef::UiResource { plugin_id, .. } = &found.tab else {
        unreachable!("already matched");
    };
    let provider = PluginId::parse(plugin_id)
        .map_err(|_| error_response(StatusCode::NOT_FOUND, "not_found", "界面的插件 ID 不对"))?;
    let resource_uri = resource_uri.to_owned();
    Ok((owner, provider, bot, found, resource_uri))
}

/// 宿主页自己的策略：它要 fetch 核心的同源接口，并放进界面的 iframe。
fn host_policy() -> String {
    "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
     img-src data:; connect-src 'self'; frame-src 'self' about: data:; form-action 'self';"
        .to_owned()
}

/// 界面的策略：不给 `'self'`，界面是 opaque origin，不能拿主人会话直接打核心接口；
/// 只按资源声明的域名放开，所有工具和资源都经宿主页。
fn frame_policy(csp: Option<&Value>) -> String {
    let domains = |key: &str| {
        csp.and_then(|csp| csp.get(key))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|value| url::Url::parse(value).ok())
            .map(|url| url.origin().ascii_serialization())
            .collect::<Vec<_>>()
    };
    let resources = domains("resourceDomains");
    let connect = domains("connectDomains");
    let frames = domains("frameDomains");
    let base = domains("baseUriDomains");
    let list = |items: &[String]| {
        if items.is_empty() {
            String::new()
        } else {
            format!(" {}", items.join(" "))
        }
    };
    format!(
        "default-src 'none'; script-src 'unsafe-inline'{}; style-src 'unsafe-inline'{}; \
         img-src data:{}; font-src{}; connect-src{}; frame-src 'self' about: data:{}; \
         base-uri{}; form-action 'self';",
        list(&resources),
        list(&resources),
        list(&resources),
        list(&resources),
        list(&connect),
        list(&frames),
        list(&base),
    )
}

/// 界面从资源内容里带的 `_meta.ui.csp`。
fn resource_csp(contents: &[ResourceContents]) -> Option<&Value> {
    let meta = match contents.first()? {
        ResourceContents::TextResourceContents { meta, .. }
        | ResourceContents::BlobResourceContents { meta, .. } => meta.as_ref()?,
        _ => return None,
    };
    meta.0.get("ui")?.get("csp")
}

fn html(contents: &[ResourceContents]) -> Result<String, String> {
    let Some(first) = contents.first() else {
        return Err("资源是空的".to_owned());
    };
    match first {
        ResourceContents::TextResourceContents {
            text, mime_type, ..
        } => {
            if !mime_type
                .as_deref()
                .is_some_and(|mime| mime.starts_with("text/html"))
            {
                return Err(format!(
                    "资源不是 HTML（{}）",
                    mime_type.as_deref().unwrap_or("没有类型")
                ));
            }
            if text.len() > MAX_RESOURCE {
                return Err("资源超过上限".to_owned());
            }
            Ok(text.clone())
        }
        _ => Err("资源不是文本".to_owned()),
    }
}

fn host_page(gateway: &Gateway, payload: Value) -> Html<String> {
    let langs = gateway.plugins.owner_languages();
    let texts = json!({
        "loading": l10n::text(&langs, "mcp-app-loading", &[]),
        "failed": l10n::text(&langs, "mcp-app-page-failed", &[]),
        "retry": l10n::text(&langs, "mcp-app-retry", &[]),
    });
    let mut page = json!({ "app": payload, "texts": texts }).to_string();
    page = page.replace('<', "\\u003c");
    Html(HOST_PAGE.replace("__PAYLOAD__", &page))
}

fn error_page(gateway: &Gateway, message: &str) -> Html<String> {
    let langs = gateway.plugins.owner_languages();
    let retry = l10n::text(&langs, "mcp-app-retry", &[]);
    let body = format!(
        "<p>{}</p><button onclick=\"location.reload()\">{}</button>",
        escape(message),
        escape(&retry),
    );
    host_page(gateway, json!({"error": body}))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(super) async fn host(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(pin): Path<String>,
) -> Response {
    if let Some(response) = unauthorized(&gateway, peer, &headers) {
        return response;
    }
    let (_, plugin, _, _, uri) = match resource_pin(&gateway, &pin) {
        Ok(found) => found,
        Err(response) => return response,
    };
    let Some(mcp) = &gateway.mcp else {
        return failed(
            &gateway,
            StatusCode::SERVICE_UNAVAILABLE,
            "核心没有接上总线",
        );
    };
    let Some(server) = gateway
        .plugins
        .mcp_servers()
        .into_iter()
        .find(|server| server.id == plugin)
    else {
        return error_page(&gateway, "提供这个界面的插件没有在运行").into_response();
    };
    let result = match mcp.bus.upstreams().read_contents(&server, &uri).await {
        Ok(result) => result,
        Err(e) => return error_page(&gateway, &e.to_string()).into_response(),
    };
    let app = match html(&result.contents) {
        Ok(app) => app,
        Err(e) => return error_page(&gateway, &e).into_response(),
    };
    let csp = resource_csp(&result.contents);
    let payload = json!({
        "pin": pin,
        "html": app,
        "csp": frame_policy(csp),
        "origin": gateway.core_origin(),
    });
    let mut response = host_page(&gateway, payload).into_response();
    let policy = host_policy();
    if let Ok(value) = policy.parse() {
        response
            .headers_mut()
            .insert("content-security-policy", value);
    }
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

pub(super) async fn tools(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Json(body): Json<PinRef>,
) -> Response {
    if let Some(response) = unauthorized(&gateway, peer, &headers) {
        return response;
    }
    let (_, plugin, _, _, _) = match resource_pin(&gateway, &body.pin) {
        Ok(found) => found,
        Err(response) => return response,
    };
    let Some(mcp) = &gateway.mcp else {
        return failed(
            &gateway,
            StatusCode::SERVICE_UNAVAILABLE,
            "核心没有接上总线",
        );
    };
    match mcp.bus.app_tools(&plugin).await {
        Ok(tools) => Json(json!({"tools": tools})).into_response(),
        Err(failure) => failed(&gateway, StatusCode::BAD_GATEWAY, failure.message),
    }
}

#[derive(Deserialize)]
pub(super) struct CallRequest {
    pin: String,
    tool: String,
    #[serde(default)]
    arguments: serde_json::Map<String, Value>,
}

pub(super) async fn call(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Json(body): Json<CallRequest>,
) -> Response {
    if let Some(response) = unauthorized(&gateway, peer, &headers) {
        return response;
    }
    let (_, plugin, _, _, _) = match resource_pin(&gateway, &body.pin) {
        Ok(found) => found,
        Err(response) => return response,
    };
    let Some(mcp) = &gateway.mcp else {
        return failed(
            &gateway,
            StatusCode::SERVICE_UNAVAILABLE,
            "核心没有接上总线",
        );
    };
    let id = crate::secret::random_token();
    gateway.app_calls.start(&id);
    let bus = mcp.bus.clone();
    let calls = gateway.app_calls.clone();
    let task_id = id.clone();
    let tool = body.tool;
    let arguments = body.arguments;
    tokio::spawn(async move {
        match bus.app_call(&plugin, &tool, arguments).await {
            Ok(result) => match serde_json::to_value(result) {
                Ok(value) => calls.finish(&task_id, value),
                Err(e) => calls.fail(&task_id, "internal".to_owned(), e.to_string()),
            },
            Err(failure) => calls.fail(&task_id, failure.kind.to_owned(), failure.message),
        }
    });
    Json(json!({"id": id})).into_response()
}

pub(super) async fn result(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(response) = unauthorized(&gateway, peer, &headers) {
        return response;
    }
    match gateway.app_calls.take(&id) {
        Some(value) => Json(value).into_response(),
        None if gateway
            .app_calls
            .calls
            .lock()
            .expect("界面调用表锁中毒")
            .contains_key(&id) =>
        {
            Json(json!({"done": false})).into_response()
        }
        None => error_response(StatusCode::NOT_FOUND, "not_found", "没有这个调用"),
    }
}

#[derive(Deserialize)]
pub(super) struct ReadRequest {
    pin: String,
    uri: String,
}

pub(super) async fn read(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Json(body): Json<ReadRequest>,
) -> Response {
    if let Some(response) = unauthorized(&gateway, peer, &headers) {
        return response;
    }
    let (_, plugin, _, _, _) = match resource_pin(&gateway, &body.pin) {
        Ok(found) => found,
        Err(response) => return response,
    };
    let Some(mcp) = &gateway.mcp else {
        return failed(
            &gateway,
            StatusCode::SERVICE_UNAVAILABLE,
            "核心没有接上总线",
        );
    };
    match mcp.bus.app_read(&plugin, &body.uri).await {
        Ok(result) => Json(result).into_response(),
        Err(failure) => failed(&gateway, StatusCode::BAD_GATEWAY, failure.message),
    }
}

#[derive(Deserialize)]
pub(super) struct MessageRequest {
    pin: String,
    text: String,
}

pub(super) async fn message(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Json(body): Json<MessageRequest>,
) -> Response {
    if let Some(response) = unauthorized(&gateway, peer, &headers) {
        return response;
    }
    let Some(matrix) = &gateway.matrix else {
        return failed(
            &gateway,
            StatusCode::SERVICE_UNAVAILABLE,
            "核心没有接上 Matrix",
        );
    };
    let Some((plugin, bot, found)) = (match find_pin(&gateway, &body.pin) {
        Ok(found) => found,
        Err(response) => return response,
    }) else {
        return error_response(StatusCode::NOT_FOUND, "not_found", "没有这个钉页");
    };
    let room = match &found.scope {
        PanelScope::Room { room_id } => room_id.clone(),
        PanelScope::Main => {
            let entry = gateway
                .plugins
                .store()
                .bots()
                .entry(&plugin, &bot)
                .ok()
                .flatten();
            let Some(room) = entry.map(|entry| entry.account.home_room) else {
                return error_response(StatusCode::NOT_FOUND, "not_found", "找不到钉页的 Bot 房间");
            };
            room
        }
    };
    let text = body.text.trim();
    if text.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "invalid_input", "消息是空的");
    }
    let login = match gateway
        .app_calls
        .owner_login(matrix, gateway.owner.username())
        .await
    {
        Ok(login) => login,
        Err(e) => return failed(&gateway, StatusCode::BAD_GATEWAY, e),
    };
    let session = matrix.session(&login);
    let sent = session
        .send_message(&room, &json!({"msgtype": "m.text", "body": text}))
        .await;
    match sent {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        // 令牌被拒（设备被删了）：清掉缓存重登一次，再发。
        Err(crate::matrix::MatrixError::Rejected { status: 401, .. }) => {
            gateway.app_calls.drop_owner_session(&login).await;
            let login = match gateway
                .app_calls
                .owner_login(matrix, gateway.owner.username())
                .await
            {
                Ok(login) => login,
                Err(e) => return failed(&gateway, StatusCode::BAD_GATEWAY, e),
            };
            let session = matrix.session(&login);
            match session
                .send_message(&room, &json!({"msgtype": "m.text", "body": text}))
                .await
            {
                Ok(_) => StatusCode::NO_CONTENT.into_response(),
                Err(e) => failed(&gateway, StatusCode::BAD_GATEWAY, e),
            }
        }
        Err(e) => failed(&gateway, StatusCode::BAD_GATEWAY, e),
    }
}

#[cfg(test)]
mod tests;
