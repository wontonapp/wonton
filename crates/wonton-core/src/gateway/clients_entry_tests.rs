//! 客户端入口（8484 上的插件主机名）的行为测试：假插件后端用 axum，原样记录收到的请求，
//! 断言转发过去的请求头和到不了后端的拒绝，不检查路由源码。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request};
use axum::response::Response;
use hyper::header;
use hyper::{Method, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tower::ServiceExt;

use super::fakes::{bind, serve};
use super::tests::{Harness, Reply, owner};
use super::{Gateway, entry_router};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::owner::OwnerStore;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::languages::Languages;
use crate::plugin::proxy_auth::ProxyAuth;
use crate::plugin::{PluginManager, PluginPackage, PluginStatus};
use crate::tunnel::Tunnel;
use wonton_tunnel::relays::RelayTable;

const ENTRY_APP: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.4'
  id: io.wonton.test.entry
  version: 1.0.0
  kind: app
  title: { zh: 入口示例, en: Entry example }
  description:
    zh: 客户端入口的行为测试用
    en: For client entry behaviour tests
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        app:
          setup_done: [sh, /wonton/setup-done.sh]
      public:
        deny: [/admin]
      auth_failures:
        - { path: /auth/login, status: [401] }
        - { path: /auth/deny, status: [403] }
  webui:
    service: app
    port: 8080
    sso: proxy
    proxy:
      headers:
        X-Remote-User: "{user}"
"#;

const HOOK_APP: &str = r#"
services:
  app:
    image: example/hook:1
x-wonton:
  spec: '0.4'
  id: io.wonton.test.hook
  version: 1.0.0
  kind: app
  title: { zh: 回调示例, en: Hook example }
  description:
    zh: 公网 allow 规则的行为测试用
    en: For public allow rule behaviour tests
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        app:
          setup_done: [sh, /wonton/setup-done.sh]
      public:
        allow: [/webhook]
"#;

/// 原样记录上游实际收到的请求。
#[derive(Debug, Clone)]
struct Received {
    method: Method,
    uri: String,
    headers: Vec<(String, String)>,
}

impl Received {
    fn has(&self, name: &str) -> bool {
        self.headers.iter().any(|(key, _)| key == name)
    }
    fn value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
    fn cookie(&self) -> String {
        self.value("cookie").unwrap_or_default().to_owned()
    }
}

struct Home {
    app: Router,
    gateway: Arc<Gateway>,
    manager: Arc<PluginManager>,
    docker: FakeDocker,
    backend: SocketAddr,
    received: Arc<Mutex<Vec<Received>>>,
    /// 假插件后端和假 Tuwunel 的任务，跟着 Home 活着。
    _backend: tokio::task::JoinHandle<()>,
    _fake: crate::matrix::fake::Fake,
    _harness: Harness,
}

impl Home {
    async fn start() -> Self {
        let h = Harness::start(None).await;
        let gateway = gateway(&h);
        let (backend, received, backend_task) = start_backend().await;
        let (fake, config) = crate::matrix::fake::start().await;
        let matrix = Arc::new(
            Matrix::connect(
                &config,
                &h.data_dir.join("clients-entry-matrix"),
                HttpClient::new(),
            )
            .await
            .unwrap(),
        );
        let mut gateway = gateway;
        let notices = Arc::new(crate::matrix::NoticeRoom::new(
            matrix.clone(),
            "owner",
            h.data_dir.join("clients-entry-notice-room"),
            Arc::new(Languages::default),
        ));
        gateway.matrix = Some(matrix);
        let gateway = Arc::new(gateway.with_notices(Some(notices.clone())));
        let manager = gateway.plugins.clone();
        let docker = h.docker.clone();
        let app = entry_router(gateway.clone());
        Self {
            app,
            gateway,
            manager,
            docker,
            backend,
            received,
            _backend: backend_task,
            _fake: fake,
            _harness: h,
        }
    }

    /// 登记一个入口插件：真的记录写入（标签、开关）+ 部署时才解析的转发地址 + 网关代登录凭据。
    /// 假 Docker 里也放上 `clients.http.service` 的容器：`setup_done` 的 exec 要求容器存在。
    fn register(&self, compose: &str, label: &str, lan: bool, public: bool) {
        let langs = Languages::from_header(None);
        let manifest = PluginPackage::manifest_of(compose, &langs).unwrap();
        let http = manifest
            .clients
            .as_ref()
            .and_then(|clients| clients.http.clone())
            .expect("清单里有 clients.http");
        self.manager.register_for_test(
            manifest.clone(),
            PluginStatus::Running,
            Some(self.backend.to_string()),
        );
        let container =
            crate::plugin::names::Instance::default().container(&manifest.id.slug(), &http.service);
        self.docker.add_image_container(&container, "example/app:1");
        if manifest
            .webui
            .as_ref()
            .is_some_and(|webui| webui.proxy.is_some())
        {
            let auth = ProxyAuth::resolve(&manifest, &Default::default(), &langs)
                .unwrap()
                .expect("sso: proxy 解析出凭据");
            self.manager.proxy_for_test(&manifest.id, auth);
        }
        self.manager
            .clients_route_for_test(&manifest.id, http, Some(self.backend.to_string()));
        self.manager
            .clients_entry_for_test(&manifest.id, label, lan, public);
    }

    fn host(&self, label: &str) -> String {
        let server = self.server();
        format!("{label}.{server}")
    }

    fn lan_host(&self, label: &str) -> String {
        let server = self.server();
        format!("{label}.lan.{server}")
    }

    fn server(&self) -> String {
        self.gateway
            .matrix
            .as_ref()
            .expect("测试里的 Matrix 服务器名")
            .server_name()
            .to_owned()
    }

    async fn request(
        &self,
        method: Method,
        host: &str,
        path: &str,
        headers: &[(&str, &str)],
    ) -> Reply {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, host);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let request = builder.body(Body::empty()).unwrap();
        Reply::of(self.app.clone().oneshot(request).await.unwrap()).await
    }

    async fn get(&self, host: &str, path: &str, headers: &[(&str, &str)]) -> Reply {
        self.request(Method::GET, host, path, headers).await
    }

    fn upstream_hits(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }
}

fn gateway(h: &Harness) -> Gateway {
    let profile = OwnerStore::for_test(&h.data_dir.join("clients-entry-owner"), &owner());
    // 不让测试连接任何中继，复用已有回环 endpoint。
    let tunnel = Tunnel::new(
        h.tunnel.devices.clone(),
        h.members.clone(),
        Arc::new(|| Languages::from_header(None)),
        h.tunnel.endpoint.clone(),
        0,
        RelayTable::from_relays(Vec::new()),
        tokio::sync::watch::channel(["192.0.2.10".parse().unwrap()].into()).1,
    );
    Gateway::new(
        crate::config::PublicUrls {
            scheme: "http".into(),
            base_domain: "core.homes.test".into(),
            port: 8080,
        },
        profile,
        "test-owner-token".into(),
        h.members.clone(),
        h.manager.clone(),
        None,
        HttpClient::new(),
        tunnel,
    )
}

/// 假插件后端：记录收到的请求；`/auth/` 开头回 401，`/spoof` 自己带计数头，`/ws` 支持协议升级。
async fn start_backend() -> (
    SocketAddr,
    Arc<Mutex<Vec<Received>>>,
    tokio::task::JoinHandle<()>,
) {
    use axum::routing::get;
    let received: Arc<Mutex<Vec<Received>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = received.clone();
    let (listener, addr) = bind().await;
    let router = Router::new()
        .route("/ws", get(upgrade_echo))
        .route(
            "/cookies",
            get(|| async {
                Response::builder()
                    .header(
                        header::SET_COOKIE,
                        "session=attacker; Domain=homes.test; Path=/",
                    )
                    .header(header::SET_COOKIE, "wonton_session=attacker; Path=/")
                    .header(header::SET_COOKIE, "theme=dark; Path=/; HttpOnly")
                    .body(Body::empty())
                    .unwrap()
            }),
        )
        .route(
            "/spoof",
            get(|| async {
                Response::builder()
                    .status(StatusCode::OK)
                    .header("x-wonton-auth-failure", "1")
                    .body(Body::from("spoofed"))
                    .unwrap()
            }),
        )
        .fallback(move |req: Request| {
            let recorded = recorded.clone();
            async move {
                let (parts, body) = req.into_parts();
                let body = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
                received_request(
                    &recorded,
                    Received {
                        method: parts.method.clone(),
                        uri: parts.uri.to_string(),
                        headers: parts
                            .headers
                            .iter()
                            .map(|(k, v)| {
                                (
                                    k.as_str().to_owned(),
                                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                                )
                            })
                            .collect(),
                    },
                );
                let status = if parts.uri.path().starts_with("/auth/") {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::OK
                };
                Response::builder()
                    .status(status)
                    .header("x-backend", "entryapp")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({"path": parts.uri.to_string(), "bytes": body.len()})
                            .to_string(),
                    ))
                    .unwrap()
            }
        });
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (addr, received, task)
}

/// 记下后端实际收到的请求。
fn received_request(received: &Mutex<Vec<Received>>, hit: Received) {
    received.lock().unwrap().push(hit);
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

fn entry_public() -> [(&'static str, &'static str); 1] {
    [("x-wonton-entry", "public")]
}

fn entry_lan() -> [(&'static str, &'static str); 1] {
    [("x-wonton-entry", "lan")]
}

#[tokio::test]
async fn only_entry_container_peer_accepted() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let server = home.server();
    let app =
        crate::entry_runtime::router_with_trusted(home.gateway.clone(), "127.0.0.1:9".to_owned());
    let ask = |peer: std::net::IpAddr, host: String| {
        let app = app.clone();
        async move {
            let mut request = Request::builder()
                .method(Method::GET)
                .uri("/.well-known/matrix/server")
                .header(header::HOST, host)
                .body(Body::empty())
                .unwrap();
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::new(peer, 40000)));
            Reply::of(app.oneshot(request).await.unwrap()).await
        }
    };
    // 入口容器的地址：过了对端检查，联邦照常回答。
    let trusted = ask(std::net::IpAddr::from([127, 0, 0, 1]), server.clone()).await;
    assert_eq!(trusted.status, StatusCode::OK);
    // 别的地址（被攻破的连接器、宿主）在这个监听上一个字节都得不到。
    let stranger = ask(
        std::net::IpAddr::from([10, 9, 9, 9]),
        format!("entryapp.{server}"),
    )
    .await;
    assert_eq!(stranger.status, StatusCode::FORBIDDEN);
    assert_eq!(stranger.json()["errcode"], "M_FORBIDDEN");
    assert!(home.upstream_hits().is_empty());
}

#[tokio::test]
async fn missing_or_double_or_mismatched_entry_header_rejected() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", true, true);
    let public = home.host("entryapp");
    let lan = home.lan_host("entryapp");
    let no_header = home.get(&public, "/", &[]).await;
    assert_eq!(no_header.status, StatusCode::BAD_REQUEST);
    assert_eq!(no_header.error_kind(), Some("invalid_input"));
    let doubled = home
        .get(
            &public,
            "/",
            &[("x-wonton-entry", "public"), ("x-wonton-entry", "lan")],
        )
        .await;
    assert_eq!(doubled.status, StatusCode::BAD_REQUEST);
    let mismatched = home.get(&public, "/", &[("x-wonton-entry", "lan")]).await;
    assert_eq!(mismatched.status, StatusCode::BAD_REQUEST);
    let mismatched_lan = home.get(&lan, "/", &[("x-wonton-entry", "public")]).await;
    assert_eq!(mismatched_lan.status, StatusCode::BAD_REQUEST);
    let wrong_value = home.get(&public, "/", &[("x-wonton-entry", "probe")]).await;
    assert_eq!(wrong_value.status, StatusCode::BAD_REQUEST);
    assert!(home.upstream_hits().is_empty());
    // 两个开关都开着、头也对得上时才转发。
    let ok = home.get(&public, "/", &entry_public()).await;
    assert_eq!(ok.status, StatusCode::OK);
    let ok_lan = home.get(&lan, "/", &entry_lan()).await;
    assert_eq!(ok_lan.status, StatusCode::OK);
}

#[tokio::test]
async fn closed_switch_rejected() {
    let home = Home::start().await;
    let id = crate::plugin::manifest::PluginId::parse("io.wonton.test.entry").unwrap();
    home.register(ENTRY_APP, "entryapp", false, false);
    let host = home.host("entryapp");
    let closed = home.get(&host, "/", &entry_public()).await;
    assert_eq!(closed.status, StatusCode::FORBIDDEN);
    assert_eq!(closed.error_kind(), Some("entry_closed"));
    assert!(home.upstream_hits().is_empty());
    // 开关是记录里的：写上再唤醒插件表，下一个请求就能过。
    home.manager
        .clients_entry_for_test(&id, "entryapp", true, true);
    let opened = home.get(&host, "/", &entry_public()).await;
    assert_eq!(opened.status, StatusCode::OK);
    assert_eq!(home.upstream_hits().len(), 1);
}

#[tokio::test]
async fn unknown_label_421() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let server = home.server();
    for host in [
        format!("ghost.{server}"),
        format!("a.b.{server}"),
        format!("{server}.evil.test"),
        format!("{server}:12345"),
    ] {
        let reply = home.get(&host, "/", &entry_public()).await;
        assert_eq!(reply.status, StatusCode::MISDIRECTED_REQUEST, "{host}");
        assert_eq!(reply.json()["errcode"], "M_UNRECOGNIZED");
    }
    assert!(home.upstream_hits().is_empty());
}

#[tokio::test]
async fn public_deny_and_allow_enforced_on_public_only() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", true, true);
    home.register(HOOK_APP, "hook", true, true);
    let public = home.host("entryapp");
    let lan = home.lan_host("entryapp");
    for path in ["/admin", "/admin/panel", "/ADMIN"] {
        let denied = home.get(&public, path, &entry_public()).await;
        assert_eq!(denied.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(denied.error_kind(), Some("public_denied"));
    }
    // 同一条路径在局域网入口不受 `public` 限制。
    let lan_ok = home.get(&lan, "/admin", &entry_lan()).await;
    assert_eq!(lan_ok.status, StatusCode::OK);
    let allowed = home.get(&public, "/public", &entry_public()).await;
    assert_eq!(allowed.status, StatusCode::OK);
    // `allow` 反过来：列表之外的路径公网进不来，局域网照常。
    let hook_public = home.host("hook");
    let hook_lan = home.lan_host("hook");
    let ok = home.get(&hook_public, "/webhook", &entry_public()).await;
    assert_eq!(ok.status, StatusCode::OK);
    let blocked = home.get(&hook_public, "/other", &entry_public()).await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
    let lan_ok = home.get(&hook_lan, "/other", &entry_lan()).await;
    assert_eq!(lan_ok.status, StatusCode::OK);
    // 到后端的正好是放行的四条；被拒的三条一条都没到。
    let hits = home.upstream_hits();
    let uris: Vec<&str> = hits.iter().map(|hit| hit.uri.as_str()).collect();
    assert_eq!(uris, vec!["/admin", "/public", "/webhook", "/other"]);
}

#[tokio::test]
async fn dot_segments_400_before_upstream() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", true, true);
    let public = home.host("entryapp");
    for path in ["/a/../b", "/a/%2e%2e/b", "/..%2fadmin", "/a/..;x/b"] {
        let reply = home.get(&public, path, &entry_public()).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{path}");
        assert_eq!(reply.error_kind(), Some("invalid_input"));
    }
    assert!(home.upstream_hits().is_empty());
    // 点段在两种入口都拒（review P1-6 裁定）：应用会各自解成点段，局域网的失败计数同样不能被绕过。
    let lan = home.lan_host("entryapp");
    let forwarded = home.get(&lan, "/a/../b", &entry_lan()).await;
    assert_eq!(forwarded.status, StatusCode::BAD_REQUEST);
    assert_eq!(forwarded.error_kind(), Some("invalid_input"));
    assert!(home.upstream_hits().is_empty());
}

#[tokio::test]
async fn setup_pending_until_setup_done_succeeds() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    let setup_done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = setup_done.clone();
    home.docker.on_exec(move |_container, argv, _stdin| {
        assert_eq!(argv.first().map(String::as_str), Some("sh"));
        if flag.load(std::sync::atomic::Ordering::SeqCst) {
            (0, String::new())
        } else {
            (1, String::new())
        }
    });
    let pending = home.get(&host, "/", &entry_public()).await;
    assert_eq!(pending.status, StatusCode::CONFLICT);
    assert_eq!(pending.error_kind(), Some("setup_pending"));
    assert!(home.upstream_hits().is_empty());
    setup_done.store(true, std::sync::atomic::Ordering::SeqCst);
    let ok = home.get(&host, "/", &entry_public()).await;
    assert_eq!(ok.status, StatusCode::OK);
    // 查到完成就缓存到插件重新部署：后续请求不再执行命令。
    let cached = home.get(&host, "/", &entry_public()).await;
    assert_eq!(cached.status, StatusCode::OK);
    assert_eq!(home.docker.execs().len(), 2);
    assert_eq!(home.upstream_hits().len(), 2);
}

#[tokio::test]
async fn setup_done_error_hides_stderr_from_anonymous() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    home.docker
        .on_exec(|_container, _argv, _stdin| (7, "database is migrating".to_owned()));
    let reply = home.get(&host, "/", &entry_public()).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), Some("plugin_unavailable"));
    // 匿名响应只带通用类别，不带 stderr 或 Docker 错误（review P0-4）；完整错误在服务器日志里。
    assert!(!reply.text().contains("database is migrating"));
    assert!(home.upstream_hits().is_empty());
}

#[tokio::test]
async fn identity_headers_and_wonton_headers_stripped() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    let reply = home
        .get(
            &host,
            "/",
            &[
                ("x-wonton-entry", "public"),
                ("x-wonton-spoofed", "1"),
                ("x-remote-user", "mallory"),
                ("x-real-ip", "203.0.113.7"),
                ("authorization", "Bearer app-own-token"),
                (
                    "cookie",
                    "app_session=keep; wonton_session=core; Wonton_X=1",
                ),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let hit = &home.upstream_hits()[0];
    assert_eq!(hit.method, Method::GET);
    assert!(!hit.has("x-remote-user"), "{hit:?}");
    assert!(!hit.has("x-wonton-spoofed"));
    assert!(!hit.has("x-wonton-entry"));
    assert_eq!(hit.value("authorization"), Some("Bearer app-own-token"));
    assert_eq!(hit.cookie(), "app_session=keep");
    // 入口写的真实来源照转，转发头由核心重写。
    assert_eq!(hit.value("x-real-ip"), Some("203.0.113.7"));
    assert_eq!(hit.value("x-forwarded-for"), Some("203.0.113.7"));
    assert_eq!(hit.value("x-forwarded-host"), Some(host.as_str()));
    assert_eq!(hit.value("x-forwarded-proto"), Some("https"));
}

#[tokio::test]
async fn webui_proxy_headers_stripped_by_normalized_name() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    home.get(
        &host,
        "/",
        &[
            ("x-wonton-entry", "public"),
            ("x_remote_user", "mallory"),
            ("X_REMOTE_USER", "mallory-again"),
        ],
    )
    .await;
    let hit = &home.upstream_hits()[0];
    assert!(!hit.has("x_remote_user"), "{hit:?}");
    assert!(!hit.has("x-remote-user"), "{hit:?}");
}

#[tokio::test]
async fn owner_cookie_never_becomes_identity() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    let reply = home
        .get(
            &host,
            "/",
            &[
                ("x-wonton-entry", "public"),
                ("cookie", "wonton_session=owner-core-session"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let hit = &home.upstream_hits()[0];
    assert_eq!(hit.cookie(), "");
    // 这个入口不代登录：没有网关代加的身份头。
    assert!(!hit.has("x-remote-user"));
    assert!(!hit.has("x-wonton-owner"));
}

#[tokio::test]
async fn auth_failure_marked_only_for_declared_paths() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    let headers: Vec<(&str, &str)> = entry_public().to_vec();
    let marked = home.get(&host, "/auth/login", &headers).await;
    assert_eq!(marked.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        marked
            .headers
            .get("x-wonton-auth-failure")
            .map(|v| v.to_str().unwrap()),
        Some("1")
    );
    // 声明了路径但状态码不在列表里、或路径没声明：都不算验证失败。
    let undeclared_status = home.get(&host, "/auth/deny", &headers).await;
    assert_eq!(undeclared_status.status, StatusCode::UNAUTHORIZED);
    assert!(
        undeclared_status
            .headers
            .get("x-wonton-auth-failure")
            .is_none()
    );
    let undeclared_path = home.get(&host, "/auth/other", &headers).await;
    assert_eq!(undeclared_path.status, StatusCode::UNAUTHORIZED);
    assert!(
        undeclared_path
            .headers
            .get("x-wonton-auth-failure")
            .is_none()
    );
    // 插件自己带的计数头先删掉，冒充不了计数。
    let spoofed = home.get(&host, "/spoof", &headers).await;
    assert_eq!(spoofed.status, StatusCode::OK);
    assert!(spoofed.headers.get("x-wonton-auth-failure").is_none());
}

#[tokio::test]
async fn core_routes_not_reachable() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    // `/_wonton/` 在插件主机名上归核心（ADR-0080），访客链接（C18）还没来：一律 404，不转发。
    for path in ["/_wonton/api/status", "/_wonton/guest/token"] {
        let reply = home.get(&host, path, &entry_public()).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(reply.error_kind(), Some("not_found"));
    }
    // Matrix 的客户端和联邦接口也不在这里：插件主机名上的请求只会去插件。
    for path in ["/_matrix/client/versions", "/_matrix/federation/v1/send/1"] {
        let reply = home.get(&host, path, &entry_public()).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}");
        assert_eq!(
            reply.headers.get("x-backend").map(|v| v.to_str().unwrap()),
            Some("entryapp")
        );
    }
    let hits = home.upstream_hits();
    assert_eq!(hits.len(), 2);
    assert!(
        hits.iter()
            .any(|hit| hit.uri.starts_with("/_matrix/client/"))
    );
    assert!(
        hits.iter()
            .any(|hit| hit.uri.starts_with("/_matrix/federation/"))
    );
}

#[tokio::test]
async fn closing_switch_drops_open_websocket() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    let (listener, addr) = bind().await;
    serve(listener, home.app.clone()).await;
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!(
        "GET /ws HTTP/1.1\r\nHost: {host}\r\nConnection: Upgrade\r\nUpgrade: echo\r\nx-wonton-entry: public\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut byte))
            .await
            .expect("等到升级响应")
            .unwrap();
        head.push(byte[0]);
    }
    assert!(
        head.starts_with(b"HTTP/1.1 101"),
        "{}",
        String::from_utf8_lossy(&head)
    );
    stream.write_all(b"ping").await.unwrap();
    let mut frame = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut frame))
        .await
        .expect("等到回显")
        .unwrap();
    assert_eq!(&frame, b"ping");
    // 关开关：记录写回 + 唤醒插件表，监视任务把入口的 watch 置位，已升级的连接立即断。
    let id = crate::plugin::manifest::PluginId::parse("io.wonton.test.entry").unwrap();
    home.manager
        .clients_entry_for_test(&id, "entryapp", false, false);
    let mut closed = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut closed)).await;
    assert!(read.is_ok(), "开关关上后连接应立即断开");
}

#[tokio::test]
async fn plugin_requests_carry_no_duplicate_host_and_lan_ports_pass() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", true, true);
    // 带端口的形式照认：公网 443、局域网 8483。
    let server = home.server();
    for (host, entry) in [
        (format!("entryapp.{server}:443"), entry_public()),
        (format!("entryapp.lan.{server}:8483"), entry_lan()),
    ] {
        let reply = home.get(&host, "/", &entry).await;
        assert_eq!(reply.status, StatusCode::OK, "{host}");
        assert_eq!(
            reply.headers.get("x-backend").map(|v| v.to_str().unwrap()),
            Some("entryapp")
        );
    }
    assert_eq!(home.upstream_hits().len(), 2);
    let first = home.upstream_hits().remove(0);
    assert_eq!(
        first.value("x-forwarded-host"),
        Some(format!("entryapp.{server}:443").as_str())
    );
}

#[path = "clients_entry_isolation_tests.rs"]
mod clients_entry_isolation_tests;

#[path = "clients_entry_race_tests.rs"]
mod clients_entry_race_tests;
