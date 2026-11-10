//! `notify.wonton.internal`：插件把通知交给核心，核心以核心账号发进主人的"Wonton"私聊（ADR-0047）。按来源地址认出
//! 插件（同 `mcp.wonton.internal`），只放行申请了 `notify` 的插件；频率超了的丢弃，并在第一次超限时告诉主人。

use std::time::Duration;

use axum::body::to_bytes;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use wonton_protocol::{NOTICE_FIELD, NoticeLevel, PluginNotice};

use super::api::error_response;
use super::{Gateway, client_addr};
use crate::mcp::card_text;
use crate::plugin::{Permission, PluginId, validate_path};

/// 请求体的上限（ADR-0047）。
const MAX_BYTES: usize = 16 * 1024;
/// `title` 是一句话，`body` 是一段话。
const MAX_TITLE: usize = 120;
const MAX_BODY: usize = 2000;
const NOTICE_PATH: &str = "/notices";

pub async fn request(gateway: &Gateway, req: Request) -> Response {
    let Some(notices) = &gateway.notices else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "matrix_unavailable",
            "核心没有接聊天服务，收不了通知",
        );
    };
    let source = client_addr(&req).and_then(|addr| gateway.plugins.source(addr.ip()));
    let source = match source {
        Some(source) if source.permissions.contains(&Permission::Notify) => source,
        Some(source) => {
            return error_response(
                StatusCode::FORBIDDEN,
                "permission_denied",
                &format!("插件 {} 没有申请 notify 权限", source.id),
            );
        }
        None => {
            return error_response(
                StatusCode::FORBIDDEN,
                "unauthenticated",
                "这个地址只给插件网络里的插件用",
            );
        }
    };
    if let Some(reject) = method_or_path(&req) {
        return reject;
    }
    let mut notice = match read_request(gateway, &source.id, req).await {
        Read::Notice(notice) => notice,
        Read::Invalid(message) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &message);
        }
    };
    notice.plugin_name = display_name(gateway, &source.id);

    if let Err(denied) = gateway.notify_limiter.admit(&source.id) {
        return drop_and_tell(gateway, notices, &notice, denied).await;
    }

    let mut body = format!("{}：{}", notice.plugin_name, notice.title);
    if let Some(text) = &notice.body {
        body.push('\n');
        body.push_str(text);
    }
    // info 按 Matrix 的推送规则不通知（.m.rule.suppress_notices），warning、critical 推送。
    let content = json!({
        "msgtype": match notice.level {
            NoticeLevel::Info => "m.notice",
            _ => "m.text",
        },
        "body": body,
        NOTICE_FIELD: notice,
    });
    match notices.send(&content).await {
        Ok(_) => StatusCode::CREATED.into_response(),
        Err(e) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "matrix_unavailable",
            &format!("发不进聊天：{e}"),
        ),
    }
}

/// 只有 `POST /notices` 收通知。
fn method_or_path(req: &Request) -> Option<Response> {
    if req.uri().path() != NOTICE_PATH {
        return Some(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            &format!("只有 {NOTICE_PATH} 收通知"),
        ));
    }
    if req.method() != Method::POST {
        return Some(error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "invalid_input",
            "这个接口只收 POST",
        ));
    }
    None
}

/// 读请求体的结果：要么是一条通知，要么是字段不合法的原因。
enum Read {
    Notice(PluginNotice),
    Invalid(String),
}

/// 读请求体、校验字段。
async fn read_request(gateway: &Gateway, id: &PluginId, req: Request) -> Read {
    let bytes = match to_bytes(req.into_body(), MAX_BYTES).await {
        Ok(bytes) => bytes,
        Err(e) => {
            return Read::Invalid(format!(
                "请求体读不了（上限 {} KiB）：{e}",
                MAX_BYTES / 1024
            ));
        }
    };
    let input: Value = match serde_json::from_slice(&bytes) {
        Ok(input) => input,
        Err(e) => return Read::Invalid(format!("请求体不是合法的 JSON：{e}")),
    };
    if !input.is_object() {
        return Read::Invalid("请求体要是一个 JSON 对象".to_owned());
    }
    match read_notice(gateway, id, &input) {
        Ok(notice) => Read::Notice(notice),
        Err(message) => Read::Invalid(message),
    }
}

/// 从请求体里读出一条通知；字段不合法的原因原样返回。插件名由调用方补上。
fn read_notice(gateway: &Gateway, id: &PluginId, input: &Value) -> Result<PluginNotice, String> {
    let string = |name: &str| -> Result<Option<String>, String> {
        match input.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(format!("{name} 要是字符串")),
        }
    };
    let title = string("title")?
        .filter(|t| !t.trim().is_empty())
        .ok_or("title 必填，一句话")?;
    if title.chars().count() > MAX_TITLE {
        return Err(format!("title 最多 {MAX_TITLE} 个字符"));
    }
    let body = string("body")?.filter(|b| !b.trim().is_empty());
    if let Some(text) = &body
        && text.chars().count() > MAX_BODY
    {
        return Err(format!("body 最多 {MAX_BODY} 个字符"));
    }
    let level = match string("level")?.as_deref() {
        None | Some("info") => NoticeLevel::Info,
        Some("warning") => NoticeLevel::Warning,
        Some("critical") => NoticeLevel::Critical,
        Some(_) => return Err("level 只能是 info、warning 或 critical".to_owned()),
    };
    let path = match string("path")? {
        None => None,
        Some(p) if p.trim().is_empty() => None,
        Some(p) => {
            if let Some(problem) = validate_path("path", &p) {
                return Err(problem);
            }
            if !gateway.plugins.has_webui(id) {
                return Err("path：这个插件没有网页，通知不能带 path".to_owned());
            }
            Some(p)
        }
    };
    Ok(PluginNotice {
        plugin_id: id.to_string(),
        plugin_name: String::new(),
        level,
        title: title.trim().to_owned(),
        body,
        path,
    })
}

/// 超了频率：这条丢弃并返回 429；第一次超限还在私聊里告诉主人，到什么时候恢复。
async fn drop_and_tell(
    gateway: &Gateway,
    notices: &crate::matrix::NoticeRoom,
    notice: &PluginNotice,
    denied: super::notify_limit::Denied,
) -> Response {
    let wait = wait_text(denied.retry_after);
    tell_limit(
        notices,
        &notice.plugin_name,
        &denied,
        &gateway.plugins.owner_languages(),
    )
    .await;
    error_response(
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limited",
        &format!(
            "通知发得太频繁（每分钟 {} 条、每天 {} 条），约 {}后再试",
            super::notify_limit::PER_MINUTE,
            super::notify_limit::PER_DAY,
            wait
        ),
    )
}

pub(crate) async fn tell_limit(
    notices: &crate::matrix::NoticeRoom,
    plugin_name: &str,
    denied: &super::notify_limit::Denied,
    languages: &crate::plugin::languages::Languages,
) {
    if let Some(body) = limit_hint(plugin_name, denied, languages)
        && let Err(error) = notices
            .send(&json!({"msgtype": "m.notice", "body": body}))
            .await
    {
        tracing::error!(%error, %plugin_name, "发通知的超限提示失败");
    }
}

pub(crate) fn limit_hint(
    plugin_name: &str,
    denied: &super::notify_limit::Denied,
    languages: &crate::plugin::languages::Languages,
) -> Option<String> {
    denied.first.then(|| {
        crate::l10n::text(
            languages,
            "chat-notify-rate-limited",
            &[
                ("plugin", plugin_name.into()),
                (
                    "seconds",
                    denied
                        .retry_after
                        .as_secs()
                        .max(1)
                        .to_string()
                        .as_str()
                        .into(),
                ),
            ],
        )
    })
}

/// 插件的显示名：清单的 title，中文优先（同确认卡片）；没有时用 ID。
fn display_name(gateway: &Gateway, id: &PluginId) -> String {
    gateway
        .plugins
        .title(id)
        .as_ref()
        .map(card_text)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| id.to_string())
}

/// 多久以后能再发，说人话：向上取整，不到一分钟是秒，不到一小时是分钟，更长是小时。
fn wait_text(wait: Duration) -> String {
    let secs = wait.as_secs();
    if secs < 60 {
        format!("{secs} 秒")
    } else if secs < 3600 {
        format!("{} 分钟", secs.div_ceil(60))
    } else {
        format!("{} 小时", secs.div_ceil(3600))
    }
}
