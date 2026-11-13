use serde_json::{Value, json};
use url::Url;
use wonton_protocol::{PlacedTab, TabRef};

use crate::appearance::AppearanceLink;

use super::super::{PagesLink, placed_tab_ref};
use super::{TabBridgeAction, TabBridgeResult, TabEnvironment, context, page_of, reply_script};

pub(crate) fn handle_message(
    tabs: &PagesLink,
    appearance: &AppearanceLink,
    page: &PlacedTab,
    is_main_frame: bool,
    origin: &str,
    message: &str,
    environment: &TabEnvironment,
) -> TabBridgeResult {
    if matches!(placed_tab_ref(page), TabRef::Url { .. }) {
        // 公网网页没有桥（ADR-0052）：壳不该把脚本注进来，注了也如实说没有桥。
        let id = serde_json::from_str::<Value>(message)
            .ok()
            .and_then(|value| value.get("id").cloned());
        return id.map_or_else(TabBridgeResult::default, |id| {
            failure(id, -32001, "Public page has no bridge")
        });
    }
    if !is_main_frame || !accepts_origin(tabs, page, origin) {
        return TabBridgeResult::default();
    }
    let value: Value = match serde_json::from_str(message) {
        Ok(value) => value,
        Err(_) => return failure(Value::Null, -32700, "Parse error"),
    };
    if let tab @ TabRef::Widget { .. } = placed_tab_ref(page)
        && super::widget::is_widget_message(&value)
    {
        // widget API 的报文原样交给驱动，应答由驱动经订阅发回（ADR-0096）。别的页面照旧当 JSON-RPC 核。
        tabs.send_to_widget(tab, message);
        return TabBridgeResult::default();
    }
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
                && page_of(tabs, page).is_ok(),
            ..TabBridgeResult::default()
        };
    };
    match request(tabs, appearance, page, &value, environment) {
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

fn accepts_origin(tabs: &PagesLink, page: &PlacedTab, origin: &str) -> bool {
    let Ok(origin) = Url::parse(origin) else {
        return false;
    };
    let Ok(current) = page_of(tabs, page) else {
        return false;
    };
    let Some(url) = current.url.as_deref().and_then(|url| Url::parse(url).ok()) else {
        return false;
    };
    super::super::media_permission::same_origin(&url, &origin)
}

type RpcError = (i32, String);
type RpcResult = Result<(Value, Option<TabBridgeAction>), RpcError>;

fn request(
    tabs: &PagesLink,
    appearance: &AppearanceLink,
    page: &PlacedTab,
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
            page_of(tabs, page).map_err(core_error)?;
            let mut payload =
                context::payload(&appearance.view(), environment).map_err(core_error)?;
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
        Some("ui/request-display-mode") => display_mode(tabs, page, params),
        Some("ui/open-link") => {
            page_of(tabs, page).map_err(core_error)?;
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
            page_of(tabs, page).map_err(core_error)?;
            let text = draft_text(params)?;
            Ok((json!({}), Some(TabBridgeAction::DraftMessage { text })))
        }
        // Wonton 自己的扩展：面板里的页要求关注（红点），ADR-0052。
        Some("ui/wonton-attention") => set_attention(tabs, page, params),
        _ => Err((-32601, "Method not found".to_owned())),
    }
}

fn display_mode(tabs: &PagesLink, page: &PlacedTab, params: &Value) -> RpcResult {
    let mode = params["mode"].as_str().ok_or_else(invalid_params)?;
    if !matches!(mode, "inline" | "fullscreen") {
        return Err(invalid_params());
    }
    // 整页就是全屏：两个取值都收，页面照样铺满，状态只回报 "fullscreen"。
    let _ = page_of(tabs, page).map_err(core_error)?;
    Ok((json!({"mode": "fullscreen"}), None))
}

/// 页面要求关注（面板那一页画红点）。栈里的页面没有红点可挂，返回输入非法。
fn set_attention(tabs: &PagesLink, page: &PlacedTab, params: &Value) -> RpcResult {
    let Some(attention) = params["attention"].as_bool() else {
        return Err(invalid_params());
    };
    tabs.set_attention(page.clone(), attention)
        .map_err(core_error)?;
    Ok((json!({}), None))
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
