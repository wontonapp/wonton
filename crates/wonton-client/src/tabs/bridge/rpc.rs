use serde_json::{Value, json};
use url::Url;
use wonton_protocol::{TabDisplayChoice, TabRef};

use crate::appearance::AppearanceLink;

use super::super::TabsLink;
use super::{TabBridgeAction, TabBridgeResult, TabEnvironment, context, page, reply_script};

pub(crate) fn handle_message(
    tabs: &TabsLink,
    appearance: &AppearanceLink,
    tab: &TabRef,
    is_main_frame: bool,
    origin: &str,
    message: &str,
    environment: &TabEnvironment,
) -> TabBridgeResult {
    if !is_main_frame || !accepts_origin(tabs, tab, origin) {
        return TabBridgeResult::default();
    }
    let value: Value = match serde_json::from_str(message) {
        Ok(value) => value,
        Err(_) => return failure(Value::Null, -32700, "Parse error"),
    };
    let id = value.get("id");
    if !value.is_object()
        || value["jsonrpc"] != "2.0"
        || !value["method"].is_string()
        || id.is_some_and(|id| !(id.is_string() || id.is_i64() || id.is_u64() || id.is_null()))
    {
        return failure(Value::Null, -32600, "Invalid Request");
    }
    // 通知不能触发请求的副作用；没有 id 就没有可回复的请求。
    let Some(id) = id else {
        return TabBridgeResult {
            initialized: value["method"] == "ui/notifications/initialized"
                && page(tabs, tab).is_ok(),
            ..TabBridgeResult::default()
        };
    };
    match request(tabs, appearance, tab, &value, environment) {
        Ok((result, action)) => TabBridgeResult {
            reply_script: Some(reply_script(
                json!({"jsonrpc":"2.0", "id":id, "result":result}),
            )),
            action,
            initialized: false,
        },
        Err((code, reason)) => failure(id.clone(), code, &reason),
    }
}

fn accepts_origin(tabs: &TabsLink, tab: &TabRef, origin: &str) -> bool {
    let Ok(origin) = Url::parse(origin) else {
        return false;
    };
    let Ok(page) = page(tabs, tab) else {
        return false;
    };
    let Some(url) = page.url.as_deref().and_then(|url| Url::parse(url).ok()) else {
        return false;
    };
    origin.scheme() == url.scheme()
        && origin.host().is_some()
        && origin.host() == url.host()
        && origin.port_or_known_default() == url.port_or_known_default()
}

type RpcError = (i32, String);
type RpcResult = Result<(Value, Option<TabBridgeAction>), RpcError>;

fn request(
    tabs: &TabsLink,
    appearance: &AppearanceLink,
    tab: &TabRef,
    message: &Value,
    environment: &TabEnvironment,
) -> RpcResult {
    let params = &message["params"];
    match message["method"].as_str() {
        Some("ui/initialize") => {
            if !params["protocolVersion"].is_string()
                || !params["appInfo"]["name"].is_string()
                || !params["appInfo"]["version"].is_string()
                || !params["appCapabilities"].is_object()
            {
                return Err(invalid_params());
            }
            let page = page(tabs, tab).map_err(core_error)?;
            let mut payload =
                context::payload(&appearance.view(), &page, environment).map_err(core_error)?;
            Ok((
                json!({
                    "protocolVersion": "2026-01-26",
                    "hostInfo": {"name":"Wonton", "version":env!("CARGO_PKG_VERSION")},
                    "hostCapabilities": {"openLinks":{}, "message":{"text":{}}},
                    "hostContext": payload["context"].take(),
                }),
                None,
            ))
        }
        Some("ui/request-display-mode") => display_mode(tabs, tab, params),
        Some("ui/open-link") => {
            page(tabs, tab).map_err(core_error)?;
            let url = params["url"].as_str().ok_or_else(invalid_params)?;
            let parsed = Url::parse(url).map_err(|_| invalid_params())?;
            if parsed.scheme() != "https" || parsed.host_str().is_none() {
                return Err(invalid_params());
            }
            Ok((
                json!({}),
                Some(TabBridgeAction::OpenLink { url: parsed.into() }),
            ))
        }
        Some("ui/message") => {
            page(tabs, tab).map_err(core_error)?;
            let text = draft_text(params)?;
            Ok((json!({}), Some(TabBridgeAction::DraftMessage { text })))
        }
        _ => Err((-32601, "Method not found".to_owned())),
    }
}

fn display_mode(tabs: &TabsLink, tab: &TabRef, params: &Value) -> RpcResult {
    let mode = params["mode"].as_str().ok_or_else(invalid_params)?;
    if !matches!(mode, "inline" | "fullscreen") {
        return Err(invalid_params());
    }
    let current = page(tabs, tab).map_err(core_error)?;
    if current.display != TabDisplayChoice::Standard {
        if mode == "fullscreen" && !current.fullscreen {
            tabs.enter_fullscreen(tab.clone()).map_err(core_error)?;
        } else if mode == "inline" && current.fullscreen {
            tabs.exit_fullscreen(tab.clone()).map_err(core_error)?;
        }
    }
    // 打开即全屏的覆盖层退出后被 A3 关闭，不再出现在 ViewModel 中。
    let view = tabs.view();
    let fullscreen = view
        .items
        .iter()
        .chain(view.overlay.iter())
        .any(|item| item.tab == *tab && item.page.as_ref().is_some_and(|page| page.fullscreen));
    Ok((
        json!({"mode": if fullscreen { "fullscreen" } else { "inline" }}),
        None,
    ))
}

fn draft_text(params: &Value) -> Result<String, RpcError> {
    if params["role"] != "user" {
        return Err(invalid_params());
    }
    // 规范原文（ext-apps 2026-01-26）的 content 是单个对象，官方 SDK 发的却是数组
    // （ext-apps#48）：两种都收，单个对象按只有一个元素处理。
    let blocks: &[Value] = match params["content"].as_array() {
        Some(array) => array,
        None => std::slice::from_ref(&params["content"]),
    };
    if blocks.is_empty() {
        return Err(invalid_params());
    }
    let mut text = String::new();
    for (index, block) in blocks.iter().enumerate() {
        if block["type"] != "text" {
            return Err(invalid_params());
        }
        let part = block["text"].as_str().ok_or_else(invalid_params)?;
        if index > 0 {
            text.push('\n');
        }
        text.push_str(part);
    }
    Ok(text)
}

fn invalid_params() -> RpcError {
    (-32602, "Invalid params".to_owned())
}

fn core_error(error: wonton_protocol::CoreError) -> RpcError {
    (-32603, error.to_string())
}

fn failure(id: Value, code: i32, message: &str) -> TabBridgeResult {
    TabBridgeResult {
        reply_script: Some(reply_script(json!({
            "jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message},
        }))),
        ..TabBridgeResult::default()
    }
}
