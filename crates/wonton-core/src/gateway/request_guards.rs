//! 网页身份和应用授权、同源跳转、写 Cookie 的小工具。

use super::*;

pub(super) fn unauthenticated(
    gateway: &Gateway,
    host: &str,
    req: &Request,
    credentials: bool,
) -> Response {
    let langs = api::languages(req.headers());
    let format = pages::Format::of(req.headers());
    if !credentials {
        return pages::error(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            &l10n::text(&langs, "gateway-cross-site-login", &[]),
            &langs,
            format,
        );
    }
    if req.method() == Method::GET && format == pages::Format::Html {
        let path = req.uri().path_and_query().map_or("/", |p| p.as_str());
        let to = format!("{}{path}", gateway.public.origin(host));
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("to", &to)
            .finish();
        return Redirect::to(&format!("{}/_wonton/sso?{query}", gateway.core_origin()))
            .into_response();
    }
    pages::error(
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
        &l10n::text(&langs, "gateway-login-required", &[]),
        &langs,
        format,
    )
}

/// 没有身份只用于已验证的 OIDC 表单回调；有身份的回调仍须检查当前角色。
pub(super) fn app_denied(
    gateway: &Gateway,
    person: Option<&Person>,
    id: &crate::plugin::PluginId,
    req: &Request,
) -> Option<Response> {
    let person = person?;
    if gateway
        .permissions(person)
        .open_apps
        .allows_plugin(id.as_str())
    {
        return None;
    }
    let langs = api::languages(req.headers());
    Some(pages::error(
        StatusCode::FORBIDDEN,
        "not_shared",
        &l10n::text(
            &langs,
            "gateway-plugin-not-shared",
            &[("id", id.as_str().into())],
        ),
        &langs,
        pages::Format::of(req.headers()),
    ))
}

/// 把跳转目标相对 `origin` 解析，只接受同源结果，返回站内路径（含查询）。浏览器会把 http(s)
/// 地址里的 `\` 当成 `/`，只拒绝 `//` 开头不够（`/\evil.example` 会跳到 `http://evil.example`）。
pub(crate) fn same_origin_path(origin: &str, target: &str) -> Option<String> {
    let base = url::Url::parse(origin).ok()?;
    let joined = base.join(target).ok()?;
    (joined.origin() == base.origin()).then(|| joined[url::Position::BeforePath..].to_owned())
}

pub(crate) fn with_cookie(mut response: Response, cookie: String) -> Response {
    if let Ok(value) = cookie.parse() {
        response.headers_mut().append(SET_COOKIE, value);
    }
    response
}
