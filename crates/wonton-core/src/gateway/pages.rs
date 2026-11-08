//! 网关自己的几个网页：登录、状态、错误。文字按浏览器请求的 `Accept-Language` 写（没有就英文，ADR-0043），
//! `lang` 属性跟着协商出的语言。错误页按 `Accept` 协商：页面导航要网页，接口请求要 JSON（[`Format`]）。

use axum::Json;
use axum::response::{Html, IntoResponse, Response};
use hyper::StatusCode;
use hyper::header::{ACCEPT, HeaderMap, HeaderName, HeaderValue};
use serde_json::json;

use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::{PluginStatus, PluginView};

/// 错误类别写在这个响应头里，方便脚本和测试判断原因而不依赖文案。
pub const ERROR_HEADER: HeaderName = HeaderName::from_static("x-wonton-error");

/// 错误响应的样子：浏览器的页面导航（`Accept` 里有 `text/html`）要看网页，其余请求（`fetch` 默认的
/// `*/*`、`application/json` 等）是代码在调用，要能读的结构化错误。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Html,
    Json,
}

impl Format {
    pub fn of(headers: &HeaderMap) -> Self {
        let wants_html = headers
            .get(ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/html"));
        if wants_html { Self::Html } else { Self::Json }
    }
}

pub fn escape(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '&' => "&amp;".to_owned(),
            '<' => "&lt;".to_owned(),
            '>' => "&gt;".to_owned(),
            '"' => "&quot;".to_owned(),
            '\'' => "&#39;".to_owned(),
            other => other.to_string(),
        })
        .collect()
}

fn layout(title: &str, body: &str, langs: &Languages) -> String {
    let lang = l10n::lang_tag(langs);
    format!(
        "<!doctype html><html lang=\"{lang}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title} · Wonton</title><style>\
         body{{font-family:system-ui,sans-serif;max-width:40rem;margin:3rem auto;padding:0 1rem;color:#222}}\
         input,button{{font-size:1rem;padding:.5rem}}input{{width:100%;box-sizing:border-box}}\
         li{{margin:.4rem 0}}code{{background:#f3f3f3;padding:0 .2rem}}.muted{{color:#777}}\
         details{{margin:.8rem 0}}summary{{cursor:pointer}}\
         pre{{white-space:pre-wrap;overflow-wrap:anywhere;background:#f3f3f3;padding:.5rem}}\
         </style></head><body>{body}</body></html>",
        title = escape(title),
        lang = escape(lang),
    )
}

pub fn error(
    status: StatusCode,
    kind: &'static str,
    message: &str,
    langs: &Languages,
    format: Format,
) -> Response {
    error_with_detail(status, kind, message, None, langs, format)
}

/// 同 [`error`]，另带一段技术详情：给人看的一句话在上面，技术细节（状态、原因）在下面。HTML 里默认收起，
/// JSON 里是单独的 `detail` 字段。
pub fn error_with_detail(
    status: StatusCode,
    kind: &'static str,
    message: &str,
    detail: Option<&str>,
    langs: &Languages,
    format: Format,
) -> Response {
    let mut response = match format {
        Format::Html => {
            let title = l10n::text(langs, "gateway-page-error-title", &[]);
            let summary = l10n::text(langs, "gateway-page-error-detail", &[]);
            let detail = detail
                .map(|detail| {
                    format!(
                        "<details><summary>{}</summary><pre>{}</pre></details>",
                        escape(&summary),
                        escape(detail)
                    )
                })
                .unwrap_or_default();
            let body = layout(
                &title,
                &format!(
                    "<h1>{}</h1><p>{}</p>{detail}<p class=\"muted\"><code>{kind}</code></p>",
                    escape(&title),
                    escape(message)
                ),
                langs,
            );
            (status, Html(body)).into_response()
        }
        Format::Json => {
            let mut error = json!({ "category": kind, "message": message });
            if let Some(detail) = detail {
                error["detail"] = detail.into();
            }
            (status, Json(json!({ "error": error }))).into_response()
        }
    };
    response
        .headers_mut()
        .insert(ERROR_HEADER, HeaderValue::from_static(kind));
    response
}

pub fn login(next: &str, failed: bool, langs: &Languages) -> Response {
    let failure = failed.then(|| {
        format!(
            "<p style=\"color:#b00\">{}</p>",
            escape(&l10n::text(langs, "gateway-login-failed", &[]))
        )
    });
    let body = format!(
        "<h1>{}</h1>\
         <p class=\"muted\">{}</p>{}\
         <form method=\"post\" action=\"/_wonton/login\">\
         <input type=\"hidden\" name=\"next\" value=\"{next}\">\
         <p><input type=\"password\" name=\"token\" autocomplete=\"current-password\" autofocus required></p>\
         <p><button type=\"submit\">{}</button></p></form>",
        escape(&l10n::text(langs, "gateway-login-heading", &[])),
        escape(&l10n::text(langs, "gateway-login-token-hint", &[])),
        failure.as_deref().unwrap_or_default(),
        escape(&l10n::text(langs, "gateway-login-submit", &[])),
        next = escape(next),
    );
    let status = if failed {
        StatusCode::UNAUTHORIZED
    } else {
        StatusCode::OK
    };
    let page = layout(&l10n::text(langs, "gateway-login-title", &[]), &body, langs);
    let mut response = (status, Html(page)).into_response();
    if failed {
        response
            .headers_mut()
            .insert(ERROR_HEADER, HeaderValue::from_static("unauthenticated"));
    }
    response
}

/// 插件的标题，按请求的语言挑（和 `Languages::pick_owned` 同一套回退）。
pub(super) fn title_of(plugin: &PluginView, langs: &Languages) -> String {
    langs.pick_owned(&plugin.title)
}

/// 插件的状态，异常时带着原因。
pub(super) fn state_of(status: &PluginStatus, langs: &Languages) -> String {
    match status {
        PluginStatus::Installing => l10n::text(langs, "gateway-state-installing", &[]),
        PluginStatus::Running => l10n::text(langs, "gateway-state-running", &[]),
        PluginStatus::Stopped => l10n::text(langs, "gateway-state-stopped", &[]),
        PluginStatus::Degraded { reason } => l10n::text(
            langs,
            "gateway-state-degraded",
            &[("reason", reason.as_str().into())],
        ),
        PluginStatus::Uninstalling => l10n::text(langs, "gateway-state-uninstalling", &[]),
        PluginStatus::Upgrading => l10n::text(langs, "gateway-state-updating", &[]),
    }
}

pub fn status(user: &str, plugins: &[PluginView], langs: &Languages) -> Response {
    let items: String = plugins
        .iter()
        .map(|p| {
            let (title, state) = (title_of(p, langs), state_of(&p.status, langs));
            let link = match &p.url {
                Some(url) => format!("<a href=\"{}\">{}</a>", escape(url), escape(&title)),
                None => escape(&title),
            };
            format!(
                "<li>{link} <span class=\"muted\">{} · {}</span></li>",
                escape(p.id.as_str()),
                escape(&state)
            )
        })
        .collect();
    let list = if items.is_empty() {
        format!(
            "<p class=\"muted\">{}</p>",
            escape(&l10n::text(langs, "gateway-status-empty", &[]))
        )
    } else {
        format!("<ul>{items}</ul>")
    };
    let body = format!(
        "<h1>Wonton</h1><p>{}</p><h2>{}</h2>{list}\
         <form method=\"post\" action=\"/_wonton/logout\"><button type=\"submit\">{}</button></form>",
        escape(&l10n::text(
            langs,
            "gateway-status-logged-in",
            &[("user", user.into())]
        )),
        escape(&l10n::text(langs, "gateway-status-apps-heading", &[])),
        escape(&l10n::text(langs, "gateway-status-logout", &[])),
        list = list,
    );
    let page = layout(
        &l10n::text(langs, "gateway-status-title", &[]),
        &body,
        langs,
    );
    Html(page).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::l10n::CATALOG;
    use http_body_util::BodyExt;

    #[test]
    fn untrusted_text_is_escaped() {
        assert_eq!(
            escape("<script>\"x\"&'y'</script>"),
            "&lt;script&gt;&quot;x&quot;&amp;&#39;y&#39;&lt;/script&gt;"
        );
    }

    #[tokio::test]
    async fn pages_follow_the_requested_language() {
        for (header, tag) in [("ja", "ja"), ("fr-FR", "en"), ("zh-TW", "zh-Hant")] {
            let langs = Languages::from_header(Some(header));
            let expected = CATALOG.negotiate(&[tag]);
            let body = body_of(login("/", false, &langs)).await;
            assert!(
                body.contains(&format!("lang=\"{tag}\"")),
                "{header}: {body}"
            );
            assert!(
                body.contains(&escape(&CATALOG.format(
                    expected,
                    "gateway-login-heading",
                    &[]
                ))),
                "{header}: {body}"
            );
        }
        // 两种请求真的给出两种语言，防止协商被绕过。
        let ja = body_of(login("/", false, &Languages::from_header(Some("ja")))).await;
        let en = body_of(login("/", false, &Languages::from_header(Some("fr-FR")))).await;
        assert_ne!(ja, en);
    }

    async fn body_of(response: Response) -> String {
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("页面响应体读得出来")
            .to_bytes();
        String::from_utf8(bytes.to_vec()).expect("页面是 UTF-8")
    }
}
