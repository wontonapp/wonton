//! 插件网页在浏览器里的隔离（`isolation`）：跨站请求不带登录状态，插件的 Cookie 出不了自己的子域名，
//! 核心只接受自己页面发起的修改请求。请求头照浏览器在各个场景下的实际行为构造。

use std::sync::atomic::Ordering;

use axum::body::Body;
use hyper::header::{self, HeaderMap};
use hyper::{Method, StatusCode};

use super::tests::{CORE, Harness, PLUGIN, TOKEN};
use super::{APP_COOKIE, CORE_COOKIE, ERROR_HEADER};

/// 另一个插件的子域名：它的页面向 [`PLUGIN`] 发请求。
pub(super) const OTHER: &str = "app-other.wonton.localhost";

/// 浏览器向插件发出的一个请求：从哪里发起、带什么 Fetch Metadata 和 `Origin`，网关该不该认人。
pub(super) struct Case {
    what: &'static str,
    pub method: Method,
    headers: Vec<(&'static str, String)>,
    pub recognized: bool,
}

impl Case {
    pub fn headers(&self) -> Vec<(&str, &str)> {
        self.headers.iter().map(|(k, v)| (*k, v.as_str())).collect()
    }

    /// 认人时请求到达插件；不认人时网关直接 401，也不跳去核心换会话（换到了也不认）。
    pub fn check(&self, status: StatusCode, headers: &HeaderMap) {
        if self.recognized {
            assert_eq!(status, StatusCode::OK, "{}", self.what);
        } else {
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{}", self.what);
            let kind = headers.get(ERROR_HEADER).map(|v| v.to_str().unwrap());
            assert_eq!(kind, Some("unauthenticated"), "{}", self.what);
        }
    }
}

/// (场景, 方法, `Sec-Fetch-Site`/`-Mode`/`-Dest`, `Origin`, 网关认不认人)。空字符串表示没有这个头；
/// `Origin` 的 `own`、`other` 是 [`PLUGIN`] 和 [`OTHER`] 的地址。
const CASES: &[(&str, Method, [&str; 3], &str, bool)] = &[
    (
        "本插件页面里的 fetch",
        Method::POST,
        ["same-origin", "cors", "empty"],
        "own",
        true,
    ),
    (
        "从地址栏或书签打开",
        Method::GET,
        ["none", "navigate", "document"],
        "",
        true,
    ),
    (
        "点了其他插件页面上的链接",
        Method::GET,
        ["same-site", "navigate", "document"],
        "",
        true,
    ),
    (
        "其他插件页面里的 iframe",
        Method::GET,
        ["same-site", "navigate", "iframe"],
        "",
        true,
    ),
    (
        "其他插件页面提交的表单",
        Method::POST,
        ["same-site", "navigate", "document"],
        "other",
        false,
    ),
    (
        "其他插件页面里的 fetch",
        Method::GET,
        ["same-site", "cors", "empty"],
        "other",
        false,
    ),
    (
        "其他插件页面里的 <img>",
        Method::GET,
        ["same-site", "no-cors", "image"],
        "",
        false,
    ),
    (
        "其他插件页面里的 <object>",
        Method::GET,
        ["same-site", "navigate", "object"],
        "",
        false,
    ),
    (
        "其他插件页面的 WebSocket",
        Method::GET,
        ["same-site", "websocket", "websocket"],
        "other",
        false,
    ),
    (
        "外站页面里的 fetch",
        Method::POST,
        ["cross-site", "cors", "empty"],
        "http://evil.example",
        false,
    ),
    // 浏览器只对 https 和 localhost 发 Fetch Metadata，局域网 http 下只有 Origin。
    (
        "没有 Fetch Metadata：本插件页面的 POST",
        Method::POST,
        ["", "", ""],
        "own",
        true,
    ),
    (
        "没有 Fetch Metadata：其他插件页面的 POST",
        Method::POST,
        ["", "", ""],
        "other",
        false,
    ),
    (
        "没有 Fetch Metadata：沙箱页面或跨站跳转后",
        Method::POST,
        ["", "", ""],
        "null",
        false,
    ),
    (
        "Fetch Metadata 和 Origin 都没有：导航、curl",
        Method::GET,
        ["", "", ""],
        "",
        true,
    ),
];

/// 同一套场景分别带 Cookie 会话（本文件）和隧道身份（`tunnel_tests`）跑。`own` 是 [`PLUGIN`] 的地址，
/// `other` 是 [`OTHER`] 的。每个请求都声明想要网页，不认人时也不能跳去换会话。
pub(super) fn browser_cases(own: &str, other: &str) -> Vec<Case> {
    let names = ["sec-fetch-site", "sec-fetch-mode", "sec-fetch-dest"];
    CASES
        .iter()
        .map(|(what, method, fetch, origin, recognized)| {
            let mut headers: Vec<(&'static str, String)> = names
                .into_iter()
                .zip(fetch)
                .filter(|(_, value)| !value.is_empty())
                .map(|(name, value)| (name, (*value).to_owned()))
                .collect();
            let origin = match *origin {
                "own" => own,
                "other" => other,
                literal => literal,
            };
            if !origin.is_empty() {
                headers.push(("origin", origin.to_owned()));
            }
            headers.push(("accept", "text/html".to_owned()));
            Case {
                what,
                method: method.clone(),
                headers,
                recognized: *recognized,
            }
        })
        .collect()
}

/// 带着这些 Cookie 的浏览器在核心上是否已登录：已登录时核心签发交接码、跳回插件，否则跳去登录页。
async fn signed_in(h: &Harness, cookie: &str) -> bool {
    let to = h.origin(PLUGIN);
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("to", &to)
        .finish();
    let reply = h
        .get(
            CORE,
            &format!("/_wonton/sso?{query}"),
            &[("cookie", cookie)],
        )
        .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    url::Url::parse(&reply.location()).is_ok_and(|to| to.host_str() == Some(PLUGIN))
}

#[tokio::test]
async fn plugin_sessions_only_count_on_its_own_pages_and_navigations() {
    let h = Harness::start(None).await;
    let app = h.app_session().await;
    let cases = browser_cases(&h.origin(PLUGIN), &h.origin(OTHER));
    for case in &cases {
        let mut headers = case.headers();
        headers.push(("cookie", &app));
        let reply = h
            .send(
                case.method.clone(),
                PLUGIN,
                "/notes",
                &headers,
                Body::empty(),
            )
            .await;
        case.check(reply.status, &reply.headers);
    }
    let recognized = cases.iter().filter(|c| c.recognized).count();
    assert_eq!(
        h.hits.load(Ordering::SeqCst),
        recognized,
        "不认人的请求不得到达插件"
    );
}

#[tokio::test]
async fn plugin_cookies_stay_on_their_own_subdomain() {
    let h = Harness::start(None).await;
    let app = h.app_session().await;
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for set_cookie in [
        "theme=dark; Domain=wonton.localhost; Path=/; HttpOnly",
        "lang=zh; domain=.LOCALHOST",
        "sid=1; Path=/; Domain=app-whoami.wonton.localhost; SameSite=Lax",
        "wonton_app=planted; Path=/",
        "WONTON_SESSION=planted; Domain=wonton.localhost",
        "=wonton_session=planted; Domain=wonton.localhost",
    ] {
        query.append_pair("c", set_cookie);
    }
    let path = format!("/set-cookie?{}", query.finish());
    let reply = h.get(PLUGIN, &path, &[("cookie", &app)]).await;
    assert_eq!(reply.status, StatusCode::OK);
    let received: Vec<&str> = reply
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap())
        .collect();
    assert_eq!(
        received,
        [
            "theme=dark; Path=/; HttpOnly",
            "lang=zh",
            "sid=1; Path=/; SameSite=Lax"
        ],
        "插件的 Cookie 去掉 Domain，网关保留的名字整条丢弃"
    );
}

#[tokio::test]
async fn cookies_planted_by_other_pages_neither_sign_in_nor_shadow_the_session() {
    let h = Harness::start(None).await;
    let app = h.app_session().await;
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    // 插件页面的脚本能用 document.cookie 给父域写 Cookie，网关拦不住；路径更长的排在真正的会话前面。
    // 非 ASCII 的 Cookie 也不能让网关读不出会话。
    let planted = format!("{APP_COOKIE}=planted; {CORE_COOKIE}=planted; name=张三; {app}");
    let reply = h.get(PLUGIN, "/notes", &[("cookie", &planted)]).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json()["headers"]["cookie"],
        "name=张三",
        "网关的 Cookie 一个都不转发，插件自己的原样转发"
    );
    let forged = h
        .get(PLUGIN, "/notes", &[("cookie", "wonton_app=planted")])
        .await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);
    assert!(signed_in(&h, &format!("{CORE_COOKIE}=planted; name=张三; {core}")).await);
    assert!(!signed_in(&h, &format!("{CORE_COOKIE}=planted")).await);
}

#[tokio::test]
async fn core_accepts_writes_only_from_its_own_pages() {
    let h = Harness::start(None).await;
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    let plugin = h.origin(PLUGIN);
    let cross_site: [&[(&str, &str)]; 4] = [
        // 插件页面里的表单、fetch，外站页面，以及没有 Fetch Metadata 的局域网 http。
        &[
            ("sec-fetch-site", "same-site"),
            ("sec-fetch-mode", "navigate"),
            ("origin", &plugin),
        ],
        &[
            ("sec-fetch-site", "same-site"),
            ("sec-fetch-mode", "cors"),
            ("origin", &plugin),
        ],
        &[
            ("sec-fetch-site", "cross-site"),
            ("origin", "http://evil.example"),
        ],
        &[("origin", &plugin)],
    ];
    for headers in cross_site {
        let mut headers = headers.to_vec();
        headers.push(("cookie", &core));
        let reply = h
            .send(
                Method::POST,
                CORE,
                "/_wonton/logout",
                &headers,
                Body::empty(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{headers:?}");
        assert_eq!(reply.error_kind(), Some("cross_site"));
    }
    assert!(signed_in(&h, &core).await, "跨站的退出请求不能让主人退出");

    // 登录表单也只接受核心自己的页面提交。
    let headers = [
        ("content-type", "application/x-www-form-urlencoded"),
        ("sec-fetch-site", "same-site"),
        ("origin", &plugin),
    ];
    let form = Body::from(format!("token={TOKEN}"));
    let login = h
        .send(Method::POST, CORE, "/_wonton/login", &headers, form)
        .await;
    assert_eq!(login.status, StatusCode::FORBIDDEN);
    assert!(login.headers.get(header::SET_COOKIE).is_none());

    // 核心自己的页面照常：有 Fetch Metadata 时看它，没有时看 Origin。
    let own = h.origin(CORE);
    let same_origin: [&[(&str, &str)]; 2] = [
        &[("sec-fetch-site", "same-origin"), ("origin", &own)],
        &[("origin", &own)],
    ];
    for headers in same_origin {
        let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
        let mut headers = headers.to_vec();
        headers.push(("cookie", &core));
        let reply = h
            .send(
                Method::POST,
                CORE,
                "/_wonton/logout",
                &headers,
                Body::empty(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER, "{headers:?}");
        assert!(!signed_in(&h, &core).await);
    }
}
