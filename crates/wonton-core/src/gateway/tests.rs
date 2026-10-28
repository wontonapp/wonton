//! 网关行为测试：走真实 TCP 连接。插件上游和 Hydra 管理接口的替身只存在于测试里。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{RawQuery, Request};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use http_body_util::BodyExt;
use hyper::header::{self, HeaderMap};
use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::tunnel_tests::loopback_tunnel;
use super::{CORE_COOKIE, ERROR_HEADER, Gateway, router};
use crate::config::{OidcProviderConfig, Owner, PublicUrls};
use crate::http::HttpClient;
use crate::oidc::Hydra;
use crate::plugin::docker::Engine;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{PluginManager, PluginPackage, PluginStatus};

pub(super) const TOKEN: &str = "test-owner-token";
pub(super) const CORE: &str = "wonton.localhost";
pub(super) const PLUGIN: &str = "app-whoami.wonton.localhost";
pub(super) const WHOAMI: &str =
    include_str!("../../../../plugins/examples/whoami/docker-compose.yml");

async fn serve(listener: TcpListener, router: Router) {
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
}

async fn bind() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    (listener, addr)
}

/// 假插件：把收到的请求（路径和请求头）原样回显；`/ws` 支持协议升级后回显字节；
/// `/set-cookie?c=..&c=..` 把每个 `c` 原样作为一条 `Set-Cookie` 返回。
pub(super) async fn echo_upstream(hits: Arc<AtomicUsize>) -> SocketAddr {
    let router = Router::new()
        .route("/ws", get(upgrade_echo))
        .route("/set-cookie", get(set_cookies))
        .fallback(move |req: Request| {
            let hits = hits.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                let headers: serde_json::Map<String, Value> = req
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.to_string(), json!(String::from_utf8_lossy(v.as_bytes()))))
                    .collect();
                Json(json!({"path": req.uri().to_string(), "headers": headers}))
            }
        });
    let (listener, addr) = bind().await;
    serve(listener, router).await;
    addr
}

async fn set_cookies(RawQuery(query): RawQuery) -> Response {
    let mut response = Response::new(Body::empty());
    for (_, value) in url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        let value = value.parse().unwrap();
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

async fn upgrade_echo(mut req: Request) -> Response {
    let on_upgrade = hyper::upgrade::on(&mut req);
    tokio::spawn(async move {
        if let Ok(upgraded) = on_upgrade.await {
            let mut io = hyper_util::rt::TokioIo::new(upgraded);
            let mut buf = [0u8; 64];
            if let Ok(n) = io.read(&mut buf).await {
                let _ = io.write_all(&buf[..n]).await;
            }
        }
    });
    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::CONNECTION, "upgrade")
        .header(header::UPGRADE, "echo")
        .body(Body::empty())
        .unwrap()
}

/// 假 Hydra 管理接口：返回预设的登录、授权请求，记录核心提交的接受内容。
pub(super) async fn fake_hydra(accepted: Arc<Mutex<Vec<Value>>>) -> SocketAddr {
    let record = |accepted: Arc<Mutex<Vec<Value>>>, redirect: &'static str| {
        move |Json(body): Json<Value>| async move {
            accepted.lock().unwrap().push(body);
            Json(json!({"redirect_to": redirect}))
        }
    };
    let router = Router::new()
        .route(
            "/admin/oauth2/auth/requests/login",
            get(|| async {
                Json(json!({"skip": false, "subject": "", "client": {"client_id": "app.whoami"}}))
            }),
        )
        .route(
            "/admin/oauth2/auth/requests/login/accept",
            axum::routing::put(record(
                accepted.clone(),
                "http://auth.wonton.localhost/after-login",
            )),
        )
        .route(
            "/admin/oauth2/auth/requests/consent",
            get(|| async {
                Json(json!({
                    "subject": "owner",
                    "client": {"client_id": "app.whoami"},
                    "requested_scope": ["openid", "profile", "email", "groups", "admin"],
                }))
            }),
        )
        .route(
            "/admin/oauth2/auth/requests/consent/accept",
            axum::routing::put(record(
                accepted,
                "http://auth.wonton.localhost/after-consent",
            )),
        );
    let (listener, addr) = bind().await;
    serve(listener, router).await;
    addr
}

pub(super) struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    text: String,
}

impl Reply {
    pub fn location(&self) -> String {
        self.headers[header::LOCATION].to_str().unwrap().to_owned()
    }

    pub fn error_kind(&self) -> Option<&str> {
        self.headers.get(ERROR_HEADER).map(|v| v.to_str().unwrap())
    }

    pub fn cookie(&self, name: &str) -> String {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap())
            .find(|v| v.starts_with(&format!("{name}=")))
            .map(|v| v.split(';').next().unwrap().to_owned())
            .unwrap_or_else(|| panic!("响应里没有 {name} Cookie：{:?}", self.headers))
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap()
    }
}

pub(super) struct Harness {
    pub addr: SocketAddr,
    pub http: HttpClient,
    pub hits: Arc<AtomicUsize>,
    pub manager: Arc<PluginManager>,
    _dir: tempfile::TempDir,
}

pub(super) fn owner() -> Owner {
    Owner {
        id: "owner".into(),
        username: "owner".into(),
        display_name: "主人".into(),
        email: "owner@wonton.localhost".into(),
    }
}

impl Harness {
    pub async fn start(hydra_admin: Option<SocketAddr>) -> Self {
        let hits = Arc::new(AtomicUsize::new(0));
        let upstream = echo_upstream(hits.clone()).await;
        let (listener, addr) = bind().await;
        let public = PublicUrls {
            scheme: "http".into(),
            base_domain: CORE.into(),
            port: addr.port(),
        };
        let dir = tempfile::tempdir().unwrap();
        let http = HttpClient::new();
        let hydra = hydra_admin.map(|admin| {
            let cfg = OidcProviderConfig {
                admin_url: format!("http://{admin}"),
                public_url: format!("http://{upstream}"),
            };
            (Hydra::new(http.clone(), &cfg), cfg.public_url)
        });
        let lifecycle = LifecycleConfig {
            public: public.clone(),
            apps_dir: dir.path().join("apps"),
            shares_dir: dir.path().join("shares"),
            core_container: None,
            tz: "UTC".into(),
            puid: 1000,
            pgid: 1000,
            install_timeout: Duration::from_secs(5),
            owner: "owner".into(),
            gateway_port: 8080,
        };
        let manager = Arc::new(PluginManager::new(
            Engine::unreachable_for_test(),
            Store::open(dir.path().join("plugins")).unwrap(),
            http.clone(),
            hydra.as_ref().map(|(h, _)| h.clone()),
            lifecycle,
        ));
        let manifest = PluginPackage::parse(WHOAMI).unwrap().manifest;
        manager.register_for_test(manifest, PluginStatus::Running, Some(upstream.to_string()));
        let gateway = Gateway::new(
            public,
            owner(),
            TOKEN.into(),
            manager.clone(),
            hydra,
            http.clone(),
            loopback_tunnel(dir.path()).await,
        );
        serve(listener, router(Arc::new(gateway))).await;
        Self {
            addr,
            http,
            hits,
            manager,
            _dir: dir,
        }
    }

    pub fn origin(&self, host: &str) -> String {
        format!("http://{host}:{}", self.addr.port())
    }

    pub async fn send(
        &self,
        method: Method,
        host: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Body,
    ) -> Reply {
        let mut builder = hyper::Request::builder()
            .method(method)
            .uri(format!("http://{}{path}", self.addr))
            .header(header::HOST, format!("{host}:{}", self.addr.port()));
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let response = self.http.send(builder.body(body).unwrap()).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    pub async fn get(&self, host: &str, path: &str, headers: &[(&str, &str)]) -> Reply {
        self.send(Method::GET, host, path, headers, Body::empty())
            .await
    }

    pub async fn login(&self, token: &str) -> Reply {
        let form = format!("token={token}&next=%2F");
        let headers = [("content-type", "application/x-www-form-urlencoded")];
        self.send(
            Method::POST,
            CORE,
            "/_wonton/login",
            &headers,
            Body::from(form),
        )
        .await
    }

    /// 走完整的交接：登录核心 → 换交接码 → 在插件域名兑换，返回插件会话 Cookie。
    pub async fn app_session(&self) -> String {
        let core = self.login(TOKEN).await.cookie(CORE_COOKIE);
        let to = format!("{}/hello?x=1", self.origin(PLUGIN));
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("to", &to)
            .finish();
        let handoff = self
            .get(CORE, &format!("/_wonton/sso?{query}"), &[("cookie", &core)])
            .await;
        assert_eq!(handoff.status, StatusCode::SEE_OTHER);
        let target = url::Url::parse(&handoff.location()).unwrap();
        assert_eq!(target.host_str(), Some(PLUGIN));
        let redeem_path = format!("{}?{}", target.path(), target.query().unwrap());
        let redeemed = self.get(PLUGIN, &redeem_path, &[]).await;
        assert_eq!(redeemed.status, StatusCode::SEE_OTHER);
        assert_eq!(redeemed.location(), "/hello?x=1");
        // 同一个交接码不能再用。
        assert_eq!(
            self.get(PLUGIN, &redeem_path, &[]).await.status,
            StatusCode::UNAUTHORIZED
        );
        redeemed.cookie(super::APP_COOKIE)
    }
}

#[tokio::test]
async fn unauthenticated_plugin_requests_never_reach_the_plugin() {
    let h = Harness::start(None).await;
    let page = h
        .get(PLUGIN, "/dashboard?id=1", &[("accept", "text/html")])
        .await;
    assert_eq!(page.status, StatusCode::SEE_OTHER);
    let to = format!("{}/dashboard?id=1", h.origin(PLUGIN));
    let expected = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("to", &to)
        .finish();
    assert_eq!(
        page.location(),
        format!("{}/_wonton/sso?{expected}", h.origin(CORE))
    );
    let api = h
        .get(PLUGIN, "/api/data", &[("accept", "application/json")])
        .await;
    assert_eq!(api.status, StatusCode::UNAUTHORIZED);
    assert_eq!(api.error_kind(), Some("unauthenticated"));
    let forged = h.get(PLUGIN, "/", &[("cookie", "wonton_app=forged")]).await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);
    assert_eq!(h.hits.load(Ordering::SeqCst), 0, "未登录的请求不得到达插件");
}

#[tokio::test]
async fn wrong_owner_token_is_rejected() {
    let h = Harness::start(None).await;
    let reply = h.login("not-the-token").await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(reply.headers.get(header::SET_COOKIE).is_none());
}

#[tokio::test]
async fn logged_in_requests_reach_plugin_without_gateway_cookies() {
    let h = Harness::start(None).await;
    let app = h.app_session().await;
    let cookies = format!("{app}; theme=dark; {CORE_COOKIE}=stolen");
    let headers = [("cookie", cookies.as_str()), ("x-forwarded-for", "6.6.6.6")];
    let reply = h.get(PLUGIN, "/hello?x=1", &headers).await;
    assert_eq!(reply.status, StatusCode::OK);
    let seen = reply.json();
    assert_eq!(seen["path"], "/hello?x=1");
    let seen = &seen["headers"];
    assert_eq!(seen["cookie"], "theme=dark", "网关 Cookie 必须被剥离");
    assert_eq!(seen["host"], format!("{PLUGIN}:{}", h.addr.port()));
    assert_eq!(
        seen["x-forwarded-host"],
        format!("{PLUGIN}:{}", h.addr.port())
    );
    assert_eq!(seen["x-forwarded-proto"], "http");
    assert_eq!(
        seen["x-forwarded-for"], "127.0.0.1",
        "伪造的转发头必须被替换"
    );
    assert_eq!(h.hits.load(Ordering::SeqCst), 1);

    // 插件会话只在签发它的子域名有效。
    let other = PluginPackage::parse(&WHOAMI.replace("app.whoami", "app.other")).unwrap();
    h.manager.register_for_test(
        other.manifest,
        PluginStatus::Running,
        Some("127.0.0.1:9".into()),
    );
    let cross = h
        .get("app-other.wonton.localhost", "/", &[("cookie", &app)])
        .await;
    assert_eq!(cross.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn websocket_style_upgrades_are_proxied() {
    let h = Harness::start(None).await;
    let app = h.app_session().await;
    let mut stream = TcpStream::connect(h.addr).await.unwrap();
    let request = format!(
        "GET /ws HTTP/1.1\r\nHost: {PLUGIN}:{}\r\nCookie: {app}\r\nConnection: Upgrade\r\nUpgrade: echo\r\n\r\n",
        h.addr.port()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(stream.read_u8().await.unwrap());
    }
    let head = String::from_utf8(head).unwrap();
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    stream.write_all(b"ping").await.unwrap();
    let mut echoed = [0u8; 4];
    stream.read_exact(&mut echoed).await.unwrap();
    assert_eq!(&echoed, b"ping");
}

#[tokio::test]
async fn redirects_stay_on_this_server() {
    let h = Harness::start(None).await;
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    for to in [
        "http://evil.example/",
        "https://app-whoami.wonton.localhost/",
        "javascript:alert(1)",
    ] {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("to", to)
            .finish();
        let reply = h
            .get(CORE, &format!("/_wonton/sso?{query}"), &[("cookie", &core)])
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{to}");
    }
    let headers = [("content-type", "application/x-www-form-urlencoded")];
    for next in [
        "http://evil.example/",
        "/\\evil.example",
        "//evil.example",
        "javascript:alert(1)",
    ] {
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("token", TOKEN)
            .append_pair("next", next)
            .finish();
        let login = h
            .send(
                Method::POST,
                CORE,
                "/_wonton/login",
                &headers,
                Body::from(form),
            )
            .await;
        assert_eq!(login.location(), "/", "{next}");
    }

    // 交接码兑换后的跳转也只能留在插件自己的域名里。
    let to = format!("{}/", h.origin(PLUGIN));
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("to", &to)
        .finish();
    let handoff = h
        .get(CORE, &format!("/_wonton/sso?{query}"), &[("cookie", &core)])
        .await;
    let target = url::Url::parse(&handoff.location()).unwrap();
    let code = target
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .unwrap();
    let crafted = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("code", &code)
        .append_pair("to", "/\\evil.example")
        .finish();
    let redeemed = h
        .get(PLUGIN, &format!("/_wonton/session?{crafted}"), &[])
        .await;
    assert_eq!(redeemed.status, StatusCode::SEE_OTHER);
    assert_eq!(redeemed.location(), "/");
}

#[tokio::test]
async fn unknown_hosts_and_unready_plugins_report_the_reason() {
    let h = Harness::start(None).await;
    assert_eq!(
        h.get("evil.example", "/", &[]).await.error_kind(),
        Some("unknown_host")
    );
    let missing = h.get("app-missing.wonton.localhost", "/", &[]).await;
    assert_eq!(missing.error_kind(), Some("not_installed"));
    let app = h.app_session().await;
    let manifest = PluginPackage::parse(WHOAMI).unwrap().manifest;
    h.manager
        .register_for_test(manifest, PluginStatus::Installing, None);
    let busy = h.get(PLUGIN, "/", &[("cookie", &app)]).await;
    assert_eq!(busy.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(busy.error_kind(), Some("not_ready"));
    let no_oidc = h
        .get(
            "auth.wonton.localhost",
            "/.well-known/openid-configuration",
            &[],
        )
        .await;
    assert_eq!(no_oidc.error_kind(), Some("oidc_unavailable"));
}

#[tokio::test]
async fn auth_host_is_forwarded_to_the_identity_provider() {
    let hydra = fake_hydra(Arc::new(Mutex::new(Vec::new()))).await;
    let h = Harness::start(Some(hydra)).await;
    let path = "/.well-known/openid-configuration?x=1";
    let reply = h.get("auth.wonton.localhost", path, &[]).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["path"], path);
}

#[tokio::test]
async fn plugin_api_requires_owner_token() {
    let h = Harness::start(None).await;
    let denied = h.get(CORE, "/_wonton/api/plugins", &[]).await;
    assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    let cookie_only = h
        .get(CORE, "/_wonton/api/plugins", &[("cookie", &core)])
        .await;
    assert_eq!(
        cookie_only.status,
        StatusCode::UNAUTHORIZED,
        "Cookie 不能调用 API"
    );
    let bearer = format!("Bearer {TOKEN}");
    let listed = h
        .get(CORE, "/_wonton/api/plugins", &[("authorization", &bearer)])
        .await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.json()["plugins"][0]["id"], "app.whoami");
    let preview = h
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/plugins/preview",
            &[("authorization", &bearer)],
            Body::from(WHOAMI),
        )
        .await;
    assert_eq!(preview.json()["url"], h.origin(PLUGIN));
    let bad = h
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/plugins/preview",
            &[("authorization", &bearer)],
            Body::from("x: ["),
        )
        .await;
    assert_eq!(bad.error_kind(), Some("invalid_input"));
}

#[tokio::test]
async fn oidc_login_and_consent_use_the_core_session() {
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let hydra = fake_hydra(accepted.clone()).await;
    let h = Harness::start(Some(hydra)).await;

    let anonymous = h
        .get(CORE, "/_wonton/oidc/login?login_challenge=abc", &[])
        .await;
    assert_eq!(anonymous.status, StatusCode::SEE_OTHER);
    assert!(anonymous.location().starts_with("/_wonton/login?next="));
    assert!(
        accepted.lock().unwrap().is_empty(),
        "未登录时不能接受 OIDC 登录"
    );

    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    let login = h
        .get(
            CORE,
            "/_wonton/oidc/login?login_challenge=abc",
            &[("cookie", &core)],
        )
        .await;
    assert_eq!(login.location(), "http://auth.wonton.localhost/after-login");
    let consent = h
        .get(
            CORE,
            "/_wonton/oidc/consent?consent_challenge=def",
            &[("cookie", &core)],
        )
        .await;
    assert_eq!(
        consent.location(),
        "http://auth.wonton.localhost/after-consent"
    );

    let accepted = accepted.lock().unwrap();
    assert_eq!(accepted[0]["subject"], "owner");
    assert_eq!(
        accepted[1]["grant_scope"],
        json!(["openid", "profile", "email", "groups"])
    );
    let claims = &accepted[1]["session"]["id_token"];
    assert_eq!(claims["email"], "owner@wonton.localhost");
    assert_eq!(claims["preferred_username"], "owner");
    assert_eq!(claims["groups"], json!(["owner"]));
}
