//! 网关自己的几个网页：登录、状态、错误。M0 只为本机浏览器验证服务，App 里不显示它们。

use axum::response::{Html, IntoResponse, Response};
use hyper::StatusCode;
use hyper::header::{HeaderName, HeaderValue};

use crate::plugin::{PluginStatus, PluginView};

/// 错误类别写在这个响应头里，方便脚本和测试判断原因而不依赖文案。
pub const ERROR_HEADER: HeaderName = HeaderName::from_static("x-wonton-error");

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

fn layout(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"zh\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title} · Wonton</title><style>\
         body{{font-family:system-ui,sans-serif;max-width:40rem;margin:3rem auto;padding:0 1rem;color:#222}}\
         input,button{{font-size:1rem;padding:.5rem}}input{{width:100%;box-sizing:border-box}}\
         li{{margin:.4rem 0}}code{{background:#f3f3f3;padding:0 .2rem}}.muted{{color:#777}}\
         </style></head><body>{body}</body></html>",
        title = escape(title)
    )
}

pub fn error(status: StatusCode, kind: &'static str, message: &str) -> Response {
    let body = layout(
        "出错了",
        &format!(
            "<h1>出错了</h1><p>{}</p><p class=\"muted\"><code>{kind}</code></p>",
            escape(message)
        ),
    );
    let mut response = (status, Html(body)).into_response();
    response
        .headers_mut()
        .insert(ERROR_HEADER, HeaderValue::from_static(kind));
    response
}

pub fn login(next: &str, failed: bool) -> Response {
    let hint = if failed {
        "<p style=\"color:#b00\">令牌不正确。</p>"
    } else {
        ""
    };
    let body = format!(
        "<h1>登录 Wonton</h1>\
         <p class=\"muted\">用令牌登录（核心数据目录下的 <code>core/owner-token</code>）；\
         App 配对上线后改为扫码。</p>{hint}\
         <form method=\"post\" action=\"/_wonton/login\">\
         <input type=\"hidden\" name=\"next\" value=\"{next}\">\
         <p><input type=\"password\" name=\"token\" autocomplete=\"current-password\" autofocus required></p>\
         <p><button type=\"submit\">登录</button></p></form>",
        next = escape(next)
    );
    let status = if failed {
        StatusCode::UNAUTHORIZED
    } else {
        StatusCode::OK
    };
    let mut response = (status, Html(layout("登录", &body))).into_response();
    if failed {
        response
            .headers_mut()
            .insert(ERROR_HEADER, HeaderValue::from_static("unauthenticated"));
    }
    response
}

/// 插件的中文名，没有就用英文名，再没有就用任意一种。
pub(super) fn title_of(plugin: &PluginView) -> String {
    let title = &plugin.title;
    title
        .get("zh")
        .or_else(|| title.get("en"))
        .or_else(|| title.values().next())
        .cloned()
        .unwrap_or_default()
}

/// 插件的状态，异常时带着原因。
pub(super) fn state_of(status: &PluginStatus) -> String {
    match status {
        PluginStatus::Installing => "安装中".to_owned(),
        PluginStatus::Running => "运行中".to_owned(),
        PluginStatus::Stopped => "已停止".to_owned(),
        PluginStatus::Degraded { reason } => format!("异常：{reason}"),
        PluginStatus::Uninstalling => "卸载中".to_owned(),
        PluginStatus::Upgrading => "升级中".to_owned(),
    }
}

pub fn status(user: &str, plugins: &[PluginView]) -> Response {
    let items: String = plugins
        .iter()
        .map(|p| {
            let (title, state) = (title_of(p), state_of(&p.status));
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
        "<p class=\"muted\">还没有安装应用。</p>".to_owned()
    } else {
        format!("<ul>{items}</ul>")
    };
    let body = format!(
        "<h1>Wonton</h1><p>已登录：{}</p><h2>插件</h2>{list}\
         <form method=\"post\" action=\"/_wonton/logout\"><button type=\"submit\">退出</button></form>",
        escape(user)
    );
    Html(layout("状态", &body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_text_is_escaped() {
        assert_eq!(
            escape("<script>\"x\"&'y'</script>"),
            "&lt;script&gt;&quot;x&quot;&amp;&#39;y&#39;&lt;/script&gt;"
        );
    }
}
