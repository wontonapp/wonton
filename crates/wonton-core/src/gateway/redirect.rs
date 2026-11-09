//! 登录跳转和 Set-Cookie：只在网关内部使用。

use axum::response::Response;
use hyper::header::SET_COOKIE;

/// 把跳转目标相对 `origin` 解析，只接受同源结果，返回站内路径（含查询）。浏览器会把 http(s)
/// 地址里的 `\` 当成 `/`，只拒绝 `//` 开头不够（`/\evil.example` 会跳到 `http://evil.example`）。
pub(super) fn same_origin_path(origin: &str, target: &str) -> Option<String> {
    let base = url::Url::parse(origin).ok()?;
    let joined = base.join(target).ok()?;
    (joined.origin() == base.origin()).then(|| joined[url::Position::BeforePath..].to_owned())
}

pub(super) fn with_cookie(mut response: Response, cookie: String) -> Response {
    if let Ok(value) = cookie.parse() {
        response.headers_mut().append(SET_COOKIE, value);
    }
    response
}
