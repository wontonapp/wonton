//! 网关行为测试：走真实 TCP 连接。插件上游和 Hydra 管理接口的替身在 [`super::fakes`] 里。

use std::net::SocketAddr;
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use http_body_util::BodyExt;
use hyper::header::{self, HeaderMap};
use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tower::ServiceExt;

use super::fakes::loopback_tunnel;
use super::fakes::{bind, echo_upstream, fake_hydra, serve};
use super::{CORE_COOKIE, ERROR_HEADER, Gateway, router};
use crate::config::{OidcProviderConfig, PublicUrls};
use crate::http::HttpClient;
use crate::oidc::Hydra;
use crate::owner::{Owner, OwnerStore};
use crate::plugin::egress::settings::Egress;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::languages::Languages;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::storefront::StoreConfig;
use crate::plugin::{PluginManager, PluginPackage, PluginStatus};
use crate::tunnel::{Tunnel, TunnelEntry};

pub(super) const TOKEN: &str = "test-owner-token";
pub(super) const CORE: &str = "wonton.localhost";
pub(super) const PLUGIN: &str = "app-whoami.wonton.localhost";
pub(super) const WHOAMI: &str =
    include_str!("../../../../plugins/examples/whoami/docker-compose.yml");

pub(super) struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub(super) text: String,
}

impl Reply {
    pub fn location(&self) -> String {
        self.headers[header::LOCATION].to_str().unwrap().to_owned()
    }

    pub fn error_kind(&self) -> Option<&str> {
        self.headers.get(ERROR_HEADER).map(|v| v.to_str().unwrap())
    }

    pub fn content_type(&self) -> &str {
        self.headers
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap())
            .unwrap_or_default()
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

    pub fn text(&self) -> &str {
        &self.text
    }

    /// 家人测试以设备的隧道身份发请求时构造：进程内 `oneshot` 的响应转成 [`Reply`]。
    pub(super) async fn of(response: axum::response::Response) -> Self {
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        Reply {
            status,
            headers,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }
}

pub(super) struct Harness {
    pub addr: SocketAddr,
    pub http: HttpClient,
    pub hits: Arc<AtomicUsize>,
    /// 假插件回显服务的地址。
    pub upstream: SocketAddr,
    pub manager: Arc<PluginManager>,
    pub docker: FakeDocker,
    /// 网关的路由：家人测试用它以设备的隧道身份发请求（进程内塞扩展，和隧道入口的做法一致）。
    pub app: Router,
    pub tunnel: Tunnel,
    /// 家人记录：和网关里用的是同一份（隧道配对也用它）。
    pub members: Arc<crate::members::Members>,
    /// 角色记录：和网关里用的是同一份，测试直接往里面塞角色。
    pub roles: Arc<crate::roles::Roles>,
    /// 网关本体：角色这类还没有接口、只在记录层验的行为直接调。
    pub gateway: Arc<Gateway>,
    pub data_dir: std::path::PathBuf,
    /// 隧道入口：配对协议（`wonton/pair/0`）在测试里也能用。
    _entry: crate::tunnel::TunnelEntry,
    _dir: tempfile::TempDir,
}

pub(super) fn owner() -> Owner {
    Owner {
        username: "owner".into(),
        display_name: "云吞".into(),
        email: Some("owner@wonton.localhost".into()),
    }
}

impl Harness {
    pub async fn start(hydra_admin: Option<SocketAddr>) -> Self {
        Self::start_full(hydra_admin, Egress::default(), None, StoreConfig::default()).await
    }

    pub async fn start_with(hydra_admin: Option<SocketAddr>, egress: Egress) -> Self {
        Self::start_full(hydra_admin, egress, None, StoreConfig::default()).await
    }

    pub async fn start_with_store(hydra_admin: Option<SocketAddr>, store: StoreConfig) -> Self {
        Self::start_full(hydra_admin, Egress::default(), None, store).await
    }

    pub async fn start_with_upgrades(
        hydra_admin: Option<SocketAddr>,
        upgrades: Option<Arc<crate::upgrade::service::Upgrades>>,
    ) -> Self {
        Self::start_full(
            hydra_admin,
            Egress::default(),
            upgrades,
            StoreConfig::default(),
        )
        .await
    }

    async fn start_full(
        hydra_admin: Option<SocketAddr>,
        egress: Egress,
        upgrades: Option<Arc<crate::upgrade::service::Upgrades>>,
        store_config: StoreConfig,
    ) -> Self {
        let hits = Arc::new(AtomicUsize::new(0));
        let upstream = echo_upstream(hits.clone()).await;
        let (listener, addr) = bind().await;
        let public = PublicUrls {
            scheme: "http".into(),
            base_domain: CORE.into(),
            port: addr.port(),
        };
        let dir = tempfile::tempdir().unwrap();
        // 数据目录的属主按当前用户派生（CI runner 的 uid 不是 1000）：目录都以进程自己的属主建，不靠 chown。
        let owner_ids = std::fs::metadata(dir.path()).unwrap();
        let http = HttpClient::new();
        let hydra = hydra_admin.map(|admin| {
            let cfg = OidcProviderConfig {
                admin_url: format!("http://{admin}"),
                public_url: format!("http://{upstream}"),
            };
            (Hydra::new(http.clone(), &cfg), cfg.public_url)
        });
        let profile = OwnerStore::for_test(dir.path(), &owner());
        let lifecycle = LifecycleConfig {
            public: public.clone(),
            data_dir: dir.path().to_owned(),
            apps_dir: dir.path().join("apps"),
            shares_dir: dir.path().join("shares"),
            relay_dir: dir.path().join("relay"),
            core_container: None,
            tz: "UTC".into(),
            puid: owner_ids.uid(),
            pgid: owner_ids.gid(),
            install_timeout: Duration::from_secs(5),
            owner: "owner".into(),
            profile: profile.clone(),
            gateway_port: 8080,
            plugin_slice: None,
            instance: Default::default(),
        };
        // 安装预览要查宿主架构，和安装一样连 Docker。
        let (docker, engine) = FakeDocker::start().await;
        let manager = Arc::new(
            PluginManager::new(
                engine,
                Store::open(dir.path().join("plugins")).unwrap(),
                http.clone(),
                hydra.as_ref().map(|(h, _)| h.clone()),
                lifecycle,
                ImageWorker::in_process(),
            )
            .with_store(store_config)
            .with_lan(super::lan_api_tests::lan())
            .with_egress(egress),
        );
        let manifest =
            PluginPackage::manifest_of(WHOAMI, &Languages::from_header(Some("zh-Hans"))).unwrap();
        manager.register_for_test(manifest, PluginStatus::Running, Some(upstream.to_string()));
        let (tunnel, members) = loopback_tunnel(dir.path()).await;
        let roles = Arc::new(crate::roles::Roles::open(dir.path().join("roles.json")).unwrap());
        let gateway = Gateway::new(
            public,
            profile,
            TOKEN.into(),
            members.clone(),
            manager.clone(),
            hydra,
            http.clone(),
            tunnel.clone(),
        )
        .with_roles(roles.clone())
        .with_upgrades(upgrades);
        let gateway = Arc::new(gateway);
        let app = router(gateway.clone());
        let entry = TunnelEntry::spawn(&tunnel, app.clone());
        serve(listener, app.clone()).await;
        Self {
            addr,
            http,
            hits,
            upstream,
            manager,
            docker,
            app,
            tunnel,
            members,
            roles,
            gateway,
            data_dir: dir.path().to_owned(),
            _entry: entry,
            _dir: dir,
        }
    }

    pub fn origin(&self, host: &str) -> String {
        format!("http://{host}:{}", self.addr.port())
    }
    pub fn data_dir(&self) -> &std::path::Path {
        self._dir.path()
    }

    pub async fn get_after_restart(
        &self,
        host: &str,
        path: &str,
        headers: &[(&str, &str)],
    ) -> Reply {
        let public = PublicUrls {
            scheme: "http".into(),
            base_domain: CORE.into(),
            port: self.addr.port(),
        };
        let gateway = Gateway::new(
            public,
            OwnerStore::for_test(self._dir.path(), &owner()),
            TOKEN.into(),
            self.members.clone(),
            self.manager.clone(),
            None,
            self.http.clone(),
            self.tunnel.clone(),
        )
        .with_roles(self.roles.clone());
        let app = router(Arc::new(gateway));
        let mut request = hyper::Request::builder()
            .method(Method::GET)
            .uri(format!("http://{}{path}", self.addr))
            .header(header::HOST, format!("{host}:{}", self.addr.port()));
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = app
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        }
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
        self.app_session_on(PLUGIN).await
    }

    /// 同 [`Self::app_session`]，插件域名是 `host`。
    pub async fn app_session_on(&self, host: &str) -> String {
        let core = self.login(TOKEN).await.cookie(CORE_COOKIE);
        let to = format!("{}/hello?x=1", self.origin(host));
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("to", &to)
            .finish();
        let handoff = self
            .get(CORE, &format!("/_wonton/sso?{query}"), &[("cookie", &core)])
            .await;
        assert_eq!(handoff.status, StatusCode::SEE_OTHER);
        let target = url::Url::parse(&handoff.location()).unwrap();
        assert_eq!(target.host_str(), Some(host));
        let redeem_path = format!("{}?{}", target.path(), target.query().unwrap());
        let redeemed = self.get(host, &redeem_path, &[]).await;
        assert_eq!(redeemed.status, StatusCode::SEE_OTHER);
        assert_eq!(redeemed.location(), "/hello?x=1");
        // 同一个交接码不能再用。
        assert_eq!(
            self.get(host, &redeem_path, &[]).await.status,
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
    // fetch 默认的 `*/*` 和显式的 `application/json` 一样：直接 401，体是 JSON，不是登录页。
    for accept in ["*/*", "application/json"] {
        let api = h.get(PLUGIN, "/api/data", &[("accept", accept)]).await;
        assert_eq!(api.status, StatusCode::UNAUTHORIZED, "{accept}");
        assert_eq!(api.error_kind(), Some("unauthenticated"), "{accept}");
        assert_eq!(api.content_type(), "application/json", "{accept}");
        assert_eq!(
            api.json()["error"]["category"],
            "unauthenticated",
            "{accept}"
        );
        assert!(
            api.json()["error"]["message"]
                .as_str()
                .is_some_and(|m| !m.is_empty()),
            "{accept}: {}",
            api.text()
        );
    }
    // 没有 `Accept`（curl、脚本）也不给网页。
    let bare = h.get(PLUGIN, "/api/data", &[]).await;
    assert_eq!(bare.status, StatusCode::UNAUTHORIZED);
    assert_eq!(bare.json()["error"]["category"], "unauthenticated");
    // 不是导航的页面地址（不带 `Accept: text/html`）照样 401。
    let forged = h.get(PLUGIN, "/", &[("cookie", "wonton_app=forged")]).await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);
    assert_eq!(forged.json()["error"]["category"], "unauthenticated");
    assert_eq!(h.hits.load(Ordering::SeqCst), 0, "未登录的请求不得到达插件");
}

#[tokio::test]
async fn upstream_responses_pass_through_untouched() {
    let h = Harness::start(None).await;
    let app = h.app_session().await;
    // 应用自己的 401：网关只转发，不改体，也不加 `x-wonton-error`，接口靠这个头区分是谁拦的。
    let reply = h.get(PLUGIN, "/unauthorized", &[("cookie", &app)]).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.error_kind(), None);
    assert_eq!(reply.json(), json!({"error": "app_login_required"}));

    // 正常响应也一样：头里只有上游自己的内容。
    let reply = h.get(PLUGIN, "/hello?x=1", &[("cookie", &app)]).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.error_kind(), None);
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
    let other = PluginPackage::manifest_of(
        &WHOAMI.replace("app.whoami", "app.other"),
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    h.manager
        .register_for_test(other, PluginStatus::Running, Some("127.0.0.1:9".into()));
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
    let manifest =
        PluginPackage::manifest_of(WHOAMI, &Languages::from_header(Some("zh-Hans"))).unwrap();
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
    let path = "/oauth2/auth?x=1";
    let reply = h.get("auth.wonton.localhost", path, &[]).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["path"], path);
}

#[tokio::test]
async fn discovery_endpoints_are_rewritten_by_who_reaches_them_under_both_names() {
    let hydra = fake_hydra(Arc::new(Mutex::new(Vec::new()))).await;
    let h = Harness::start(Some(hydra)).await;
    let port = h.addr.port();
    for host in ["auth.wonton.localhost", "auth.wonton.internal"] {
        let reply = h.get(host, "/.well-known/openid-configuration", &[]).await;
        assert_eq!(reply.status, StatusCode::OK, "{host}");
        let browser = format!("http://auth.wonton.localhost:{port}");
        let backend = format!("http://auth.wonton.internal:{port}");
        assert_eq!(
            reply.json(),
            json!({
                "issuer": "http://auth.wonton.internal",
                "authorization_endpoint": format!("{browser}/oauth2/auth"),
                "token_endpoint": format!("{backend}/oauth2/token"),
                "userinfo_endpoint": format!("{backend}/userinfo"),
                "jwks_uri": format!("{backend}/.well-known/jwks.json"),
                "revocation_endpoint": format!("{backend}/oauth2/revoke"),
                "end_session_endpoint": format!("{browser}/oauth2/sessions/logout"),
                "scopes_supported": ["openid"],
            }),
            "{host}"
        );
    }
}

#[tokio::test]
async fn only_the_plugins_own_network_reaches_it_without_a_session() {
    let unauthenticated = &[("accept", "application/json")];
    // 别的插件网络里的容器访问它。
    let h = Harness::start(None).await;
    h.manager
        .register_source_for_test("app.other", "127.0.0.1/32", vec![]);
    let other = h.get(PLUGIN, "/", unauthenticated).await;
    assert_eq!(other.error_kind(), Some("unauthenticated"));

    // 它的容器访问别的插件。
    let h = Harness::start(None).await;
    super::proxy_sso_tests::register_vault(&h.manager).await;
    h.manager
        .register_source_for_test("app.whoami", "127.0.0.1/32", vec![]);
    let elsewhere = h
        .get(super::proxy_sso_tests::VAULT_HOST, "/", unauthenticated)
        .await;
    assert_eq!(elsewhere.error_kind(), Some("unauthenticated"));

    // 不在任何插件网络里的来源。
    let h = Harness::start(None).await;
    let outside = h.get(PLUGIN, "/", unauthenticated).await;
    assert_eq!(outside.error_kind(), Some("unauthenticated"));
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
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/examples/whoami");
    let package = PluginPackage::load_dir(&dir, &Languages::from_header(Some("zh-Hans"))).unwrap();
    let preview = h
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/plugins/preview",
            &[
                ("authorization", &bearer),
                ("content-type", "application/x-tar"),
            ],
            Body::from(package.archive()),
        )
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text);
    let preview = preview.json();
    let blocked = preview["blocked"].as_str().unwrap();
    // 请求没带 Accept-Language，回退英文；期望值用目录本身格式化出来。
    let expected = crate::l10n::text(
        &crate::plugin::languages::Languages::default(),
        "install-already-installed",
        &[
            ("id", "app.whoami".into()),
            ("version", package.manifest.version.as_str().into()),
        ],
    );
    assert_eq!(blocked, expected);
    assert!(preview["digest"].as_str().unwrap().starts_with("sha256:"));
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

    // 引用包内配置的上传必须带完整插件包，只有 compose 文本时应拒绝。
    let compose = "services:\n  app:\n    image: nginx\n    configs: [settings]\nconfigs:\n  settings:\n    file: config.yaml\nx-wonton:\n  spec: '0.4'\n  id: app.config-upload\n  version: 1.0.0\n  kind: app\n  title: {zh: 配置, en: Config}\n  description: {zh: 配置上传, en: Config upload}\n  permissions: []\n";
    let files = [("config.yaml".to_owned(), b"enabled: true\n".to_vec())].into();
    let package = PluginPackage::with_files(compose, files, &Languages::default()).unwrap();
    let archive = package.archive();
    let tar = [
        ("authorization", bearer.as_str()),
        ("content-type", "application/x-tar"),
    ];
    let preview = h
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/plugins/preview",
            &tar,
            Body::from(archive),
        )
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text);
    assert_eq!(preview.json()["id"], "app.config-upload");
    // 只有 compose 文本时，引用的 config.yaml 缺失。
    let text = h
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/plugins/preview",
            &[("authorization", &bearer)],
            Body::from(compose),
        )
        .await;
    assert_eq!(text.error_kind(), Some("invalid_input"));
}

#[tokio::test]
async fn plugin_api_answers_in_the_requested_language() {
    let h = Harness::start(None).await;
    let bearer = format!("Bearer {TOKEN}");
    let english = [
        ("authorization", bearer.as_str()),
        ("accept-language", "en-GB, zh;q=0.5"),
    ];
    let listed = h.get(CORE, "/_wonton/api/plugins", &english).await.json();
    let whoami = &listed["plugins"][0];
    assert_eq!(whoami["title"], "Who am I");
    assert_eq!(
        whoami["status"],
        json!({"state": "running", "health": "unknown"})
    );
    assert_eq!(whoami["web_url"], format!("{}/", h.origin(PLUGIN)));
    assert_eq!(whoami["trust"], "local");

    let detail = h
        .get(CORE, "/_wonton/api/plugins/app.whoami", &english)
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.text);
    assert_eq!(detail.json()["installed"]["id"], "app.whoami");
    let missing = h
        .get(CORE, "/_wonton/api/plugins/app.missing", &english)
        .await;
    assert_eq!(missing.error_kind(), Some("not_installed"));
    let image = format!("/_wonton/api/images/{}/icon-192.png", "0".repeat(64));
    assert_eq!(
        h.get(CORE, &image, &english).await.status,
        StatusCode::NOT_FOUND
    );

    // JSON 请求体是按来源预览和安装，不当成 compose 文本去解析：来源写错、少了摘要都是请求不对。
    let json = [
        ("authorization", bearer.as_str()),
        ("content-type", "application/json"),
    ];
    for path in ["/_wonton/api/plugins/preview", "/_wonton/api/plugins"] {
        let body = Body::from(r#"{"source": "github.com/example/plugin"}"#);
        let reply = h.send(Method::POST, CORE, path, &json, body).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{path}");
        assert_eq!(reply.error_kind(), Some("invalid_input"), "{path}");
    }
}
