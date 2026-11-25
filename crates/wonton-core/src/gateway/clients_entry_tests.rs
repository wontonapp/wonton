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
        Arc::new(crate::relay_settings::RelaySettings::for_table(
            RelayTable::from_relays(Vec::new()),
        )),
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
    .with_public_access(true)
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

#[path = "clients_entry_admission_tests.rs"]
mod clients_entry_admission_tests;

#[path = "clients_entry_forwarding_tests.rs"]
mod clients_entry_forwarding_tests;

#[path = "clients_entry_headers_tests.rs"]
mod clients_entry_headers_tests;

#[path = "clients_entry_isolation_tests.rs"]
mod clients_entry_isolation_tests;

#[path = "clients_entry_policy_tests.rs"]
mod clients_entry_policy_tests;

#[path = "clients_entry_proxy_auth_tests.rs"]
mod clients_entry_proxy_auth_tests;

#[path = "clients_entry_race_tests.rs"]
mod clients_entry_race_tests;

#[path = "clients_entry_setup_tests.rs"]
mod clients_entry_setup_tests;

#[path = "clients_entry_credentials_tests.rs"]
mod clients_entry_credentials_tests;
