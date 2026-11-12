//! `notify.wonton.internal`：插件把通知交给核心，核心以核心账号发进主人的"Wonton"私聊（ADR-0047）。按来源地址认出
//! 插件（同 `mcp.wonton.internal`），只放行申请了 `notify` 的插件；频率超了的丢弃，并在第一次超限时告诉主人。
//!
//! 除了自己那套 `{title, body, level, path}`，还收上游常见的写法：ntfy 的纯文本正文配 `Title`、`Priority`、`Tags`
//! 头，Gotify 的 `{title, message, priority}`。其余 JSON 拍平成可读文本；没有标题时从载荷里取，不再要求必填。
//! 插件把上游应用的通知地址直接填成 `WONTON_NOTIFY_URL` 就能用，路径不看（ntfy 的主题、Gotify 的 `/message`）。

use std::time::Duration;

use axum::Json;
use axum::body::to_bytes;
use axum::extract::Request;
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use hyper::{Method, StatusCode};
use serde_json::{Map, Value, json};
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
/// 上游的字段名里能当标题的，按这个顺序找。
const TITLE_KEYS: [&str; 4] = ["title", "subject", "name", "summary"];
/// 已经在上面取过值的键：拍平的正文里不再重复。
const USED_KEYS: [&str; 9] = [
    "title", "subject", "body", "message", "text", "level", "priority", "path", "tags",
];
/// 头部带来的标签最多留几个。
const MAX_TAGS: usize = 10;

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
    if let Some(reject) = method(&req) {
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
        // 上游的通用 Webhook 只认 200（青龙把 201 记成失败）；正文给 `{}`，ntfy、Gotify 的客户端会解 JSON。
        Ok(_) => (StatusCode::OK, Json(json!({}))).into_response(),
        Err(e) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "matrix_unavailable",
            &format!("发不进聊天：{e}"),
        ),
    }
}

/// ntfy 默认用 `PUT` 发、Gotify 用 `POST`；路径当主题用，不看。
fn method(req: &Request) -> Option<Response> {
    if req.method() == Method::POST || req.method() == Method::PUT {
        return None;
    }
    Some(error_response(
        StatusCode::METHOD_NOT_ALLOWED,
        "invalid_input",
        "这个接口只收 POST 和 PUT",
    ))
}

/// 读请求体的结果：要么是一条通知，要么是载荷不合法的原因。
enum Read {
    Notice(PluginNotice),
    Invalid(String),
}

/// 读请求体：`application/json` 按 JSON 读，其余整段当纯文本；带 JSON 内容类型但解析不了的也当纯文本。
async fn read_request(gateway: &Gateway, id: &PluginId, req: Request) -> Read {
    let (parts, body) = req.into_parts();
    let headers = Headers::read(&parts.headers);
    let json = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("application/json"));
    let bytes = match to_bytes(body, MAX_BYTES).await {
        Ok(bytes) => bytes,
        Err(e) => {
            return Read::Invalid(format!(
                "请求体读不了（上限 {} KiB）：{e}",
                MAX_BYTES / 1024
            ));
        }
    };
    let draft = match json.then(|| serde_json::from_slice::<Value>(&bytes).ok()) {
        Some(Some(value)) => read_json(&value, headers),
        Some(None) => read_text(&bytes, headers),
        None => read_text(&bytes, headers),
    };
    match draft.and_then(|draft| notice(gateway, id, draft)) {
        Ok(notice) => Read::Notice(notice),
        Err(message) => Read::Invalid(message),
    }
}

/// 上报的载荷里能对上 ntfy、Gotify 的同义字段。
#[derive(Default)]
struct Headers {
    title: Option<String>,
    /// ntfy 的优先级：1 最低，5 最高。
    priority: Option<u64>,
    tags: Vec<String>,
}

impl Headers {
    /// 值读不出来就当没有：这些头是上游客户端写的，不该让整条通知失败。
    fn read(headers: &HeaderMap) -> Self {
        // `to_str` 只收可见 ASCII；中文标题按 UTF-8 解。
        let first = |names: &[&str]| {
            names.iter().find_map(|name| {
                let text = String::from_utf8_lossy(headers.get(*name)?.as_bytes());
                let text = text.trim().to_owned();
                (!text.is_empty()).then_some(text)
            })
        };
        Self {
            title: first(&["title", "x-title"]),
            priority: first(&["priority", "x-priority"]).and_then(|value| value.parse().ok()),
            tags: first(&["tags", "x-tags"])
                .map(|tags| split_tags(&tags))
                .unwrap_or_default(),
        }
    }
}

/// 载荷里读出来的内容，还没检查必填和长度。
#[derive(Default)]
struct Draft {
    title: Option<String>,
    body: Option<String>,
    level: Option<NoticeLevel>,
    path: Option<String>,
    tags: Vec<String>,
}

/// 从 JSON 里读：自己的字段优先，其余从上游的写法里取；一个能读的字段都没有时把整段 JSON 拍平成正文。
fn read_json(value: &Value, headers: Headers) -> Result<Draft, String> {
    let ntfy_scale = headers.priority.is_some();
    let mut draft = Draft {
        level: headers
            .priority
            .map(|priority| priority_level(priority, true)),
        tags: headers.tags,
        ..Default::default()
    };
    if let Value::Object(map) = value {
        draft.title = explicit_title(map)?;
        draft.body = explicit_body(map)?;
        draft.level = level(map, ntfy_scale)?.or(draft.level);
        draft.path = field(map, &["path"])?;
        if let Some(tags) = map.get("tags") {
            draft.tags.extend(tags_of(tags));
        }
    }
    let mut lines = Vec::new();
    render(value, "", &mut lines);
    if draft.title.is_none() {
        draft.title = find_title(value);
    }
    match &draft.body {
        None => {
            // 正文要自己拍平：第一行当标题，剩下的才是正文。
            if draft.title.is_none() && !lines.is_empty() {
                draft.title = Some(lines.remove(0));
            }
            if !lines.is_empty() {
                draft.body = Some(lines.join("\n"));
            }
        }
        Some(body) if draft.title.is_none() => {
            let (first, rest) = body.split_once('\n').unwrap_or((body, ""));
            draft.title = Some(one_line(first)).filter(|title| !title.is_empty());
            draft.body = Some(rest.trim().to_owned()).filter(|rest| !rest.is_empty());
        }
        Some(_) => {}
    }
    Ok(draft)
}

/// 纯文本载荷（ntfy 的写法）：`Title` 头是标题，没有时第一行当标题、剩下的当正文。
fn read_text(bytes: &[u8], headers: Headers) -> Result<Draft, String> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    let mut draft = Draft {
        level: headers
            .priority
            .map(|priority| priority_level(priority, true)),
        tags: headers.tags,
        ..Default::default()
    };
    match headers.title {
        Some(title) => {
            draft.title = Some(one_line(&title)).filter(|title| !title.is_empty());
            draft.body = Some(text.to_owned()).filter(|text| !text.is_empty());
        }
        None => {
            let (title, body) = text.split_once('\n').unwrap_or((text, ""));
            draft.title = Some(one_line(title)).filter(|title| !title.is_empty());
            draft.body = Some(body.trim().to_owned()).filter(|body| !body.is_empty());
        }
    }
    Ok(draft)
}

/// 合成一条通知：补标题、查网页；自己填的字段已经查过长度，从载荷里取来的截断。
fn notice(gateway: &Gateway, id: &PluginId, draft: Draft) -> Result<PluginNotice, String> {
    let title = draft
        .title
        .map(|title| fit(&title, MAX_TITLE))
        .unwrap_or_else(|| "通知".to_owned());
    let mut body = draft.body.map(|body| fit(&body, MAX_BODY));
    if !draft.tags.is_empty() {
        let tags = draft
            .tags
            .iter()
            .map(|tag| fit(tag, 40))
            .collect::<Vec<_>>()
            .join("、");
        let text = body.get_or_insert_default();
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!("标签：{tags}"));
        body = body.map(|body| fit(&body, MAX_BODY));
    }
    let path = match draft.path {
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
        level: draft.level.unwrap_or(NoticeLevel::Info),
        title,
        body,
        path,
    })
}

/// 自己格式的标题：压成一行，超长算错。
fn explicit_title(map: &Map<String, Value>) -> Result<Option<String>, String> {
    let title = field(map, &["title", "subject"])?;
    let title = title
        .map(|title| one_line(&title))
        .filter(|title| !title.is_empty());
    check_len(title, MAX_TITLE, "title")
}

/// 自己格式的正文：超长算错。
fn explicit_body(map: &Map<String, Value>) -> Result<Option<String>, String> {
    let body = field(map, &["body", "message", "text"])?;
    let body = body.filter(|body| !body.trim().is_empty());
    check_len(body, MAX_BODY, "body")
}

/// 取一个字符串字段：没有或空串是 `None`，不是字符串是错误。
fn field(map: &Map<String, Value>, names: &[&str]) -> Result<Option<String>, String> {
    for name in names {
        match map.get(*name) {
            None | Some(Value::Null) => {}
            Some(Value::String(text)) if text.trim().is_empty() => {}
            Some(Value::String(text)) => return Ok(Some(text.clone())),
            Some(_) => return Err(format!("{name} 要是字符串")),
        }
    }
    Ok(None)
}

/// 级别：自己格式的 `level`，或上游的 `priority`（ntfy 是 1–5，Gotify 是 0–10）。
fn level(map: &Map<String, Value>, ntfy_scale: bool) -> Result<Option<NoticeLevel>, String> {
    if let Some(name) = field(map, &["level"])? {
        return named_level(&name).map(Some);
    }
    match map.get("priority") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(name)) => named_level(name).map(Some),
        Some(Value::Number(number)) => Ok(number
            .as_u64()
            .map(|priority| priority_level(priority, ntfy_scale))),
        Some(_) => Err("priority 要是数字或级别名".to_owned()),
    }
}

fn named_level(name: &str) -> Result<NoticeLevel, String> {
    match name.trim().to_ascii_lowercase().as_str() {
        "info" | "low" | "min" | "default" | "normal" => Ok(NoticeLevel::Info),
        "warning" | "high" => Ok(NoticeLevel::Warning),
        "critical" | "max" | "urgent" => Ok(NoticeLevel::Critical),
        _ => Err("level 只能是 info、warning、critical（上游的 low、high、max 也行）".to_owned()),
    }
}

fn priority_level(priority: u64, ntfy_scale: bool) -> NoticeLevel {
    if ntfy_scale {
        match priority {
            0..=3 => NoticeLevel::Info,
            4 => NoticeLevel::Warning,
            _ => NoticeLevel::Critical,
        }
    } else {
        match priority {
            0..=7 => NoticeLevel::Info,
            8..=9 => NoticeLevel::Warning,
            _ => NoticeLevel::Critical,
        }
    }
}

/// 把任意 JSON 拍成“键: 值”行；数组里的标量并成一行，对象和混合数组按层展开。
fn render(value: &Value, prefix: &str, lines: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if prefix.is_empty() && USED_KEYS.contains(&key.as_str()) {
                    continue;
                }
                let key = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                render(value, &key, lines);
            }
        }
        Value::Array(items)
            if items
                .iter()
                .all(|item| !item.is_object() && !item.is_array()) =>
        {
            let joined = items
                .iter()
                .filter_map(scalar)
                .collect::<Vec<_>>()
                .join("、");
            if !joined.is_empty() {
                lines.push(line(prefix, &joined));
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                render(item, &format!("{prefix}[{index}]"), lines);
            }
        }
        other => {
            if let Some(text) = scalar(other) {
                lines.push(line(prefix, &text));
            }
        }
    }
}

fn line(prefix: &str, value: &str) -> String {
    if prefix.is_empty() {
        value.to_owned()
    } else {
        format!("{prefix}: {value}")
    }
}

/// 标量的可读形式；空串和 null 没有。
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_owned()),
        Value::String(_) | Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// 从任意层找一个能当标题的字段，先找到先用。
fn find_title(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => TITLE_KEYS
            .iter()
            .find_map(|key| match map.get(*key) {
                Some(Value::String(text)) if !text.trim().is_empty() => Some(one_line(text)),
                _ => None,
            })
            .or_else(|| map.values().find_map(find_title)),
        Value::Array(items) => items.iter().find_map(find_title),
        _ => None,
    }
}

fn tags_of(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str())
            .flat_map(split_tags)
            .collect(),
        Value::String(text) => split_tags(text),
        _ => Vec::new(),
    }
}

fn split_tags(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .take(MAX_TAGS)
        .map(str::to_owned)
        .collect()
}

/// 标题是一行：换行和连续的空白并成一个空格。
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn check_len(text: Option<String>, max: usize, name: &str) -> Result<Option<String>, String> {
    match text {
        Some(text) if text.chars().count() > max => Err(format!("{name} 最多 {max} 个字符")),
        text => Ok(text),
    }
}

/// 超长的上游内容截断到上限，末尾留省略号。
fn fit(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
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
