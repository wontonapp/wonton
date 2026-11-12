//! 经隧道的网关行为测试：真实的 iroh 隧道（只走回环地址）、设备本地代理（和 App 一样带会话凭据）和
//! 网关。插件上游和 Hydra 管理接口的替身与 `tests.rs` 共用。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::plugin::languages::Languages;
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use http_body_util::BodyExt;
use hyper::header::{self, HeaderMap};
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::Value;
use tower::ServiceExt;

use wonton_tunnel::{
    LocalProxy, ProxyAccess, ProxyCredential, TunnelClient, TunnelError, TunnelStatus,
};

use super::fakes::{echo_upstream, fake_hydra, loopback_endpoint, loopback_tunnel_with};
use super::isolation_tests::{OTHER, browser_cases};
use super::proxy_sso_tests::{KEY, VAULT_HOST, leaked, received, register_vault};
use super::tests::{CORE, PLUGIN, TOKEN, WHOAMI, owner};
use super::{Gateway, router};
use crate::config::{OidcProviderConfig, PublicUrls};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::oidc::Hydra;
use crate::owner::OwnerStore;
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{PluginManager, PluginPackage, PluginStatus};
use crate::tunnel::{Tunnel, TunnelEntry};

/// 浏览器看到的端口：经隧道时不监听，只用来拼地址。
const PORT: u16 = 18480;

pub(super) struct Home {
    pub(super) app: Router,
    pub(super) plugins: Arc<PluginManager>,
    pub(super) tunnel: Tunnel,
    pub(super) members: Arc<crate::members::Members>,
    hits: Arc<AtomicUsize>,
    _entry: TunnelEntry,
    _dir: tempfile::TempDir,
}

impl Home {
    pub(super) async fn start(hydra_admin: Option<SocketAddr>) -> Self {
        Self::start_with(hydra_admin, None).await
    }
    pub(super) async fn start_lan(addresses: &[&str]) -> Self {
        let lan: std::collections::BTreeSet<std::net::IpAddr> =
            addresses.iter().map(|a| a.parse().unwrap()).collect();
        Self::start_full(None, None, lan, None, None).await
    }

    pub(super) async fn start_with(
        hydra_admin: Option<SocketAddr>,
        matrix: Option<Arc<Matrix>>,
    ) -> Self {
        Self::start_full(
            hydra_admin,
            matrix,
            std::collections::BTreeSet::from(["192.168.1.2".parse().unwrap()]),
            None,
            None,
        )
        .await
    }
    pub(super) async fn start_with_friends(
        friends: Arc<crate::friends::Friends>,
        blocks: Option<Arc<crate::matrix::FederationBlocks>>,
    ) -> Self {
        Self::start_full(
            None,
            None,
            std::collections::BTreeSet::from(["192.168.1.2".parse().unwrap()]),
            Some(friends),
            blocks,
        )
        .await
    }
    pub(super) fn owner_device(&self) -> iroh::EndpointId {
        let id = SecretKey::generate().public();
        self.tunnel.devices.add(id, None).unwrap();
        id
    }

    pub(super) fn member_device(&self, username: &str) -> iroh::EndpointId {
        let id = SecretKey::generate().public();
        self.tunnel
            .devices
            .add_if(id, None, Some(username.to_owned()), |_| true)
            .unwrap();
        id
    }

    async fn start_full(
        hydra_admin: Option<SocketAddr>,
        matrix: Option<Arc<Matrix>>,
        lan: std::collections::BTreeSet<std::net::IpAddr>,
        friends: Option<Arc<crate::friends::Friends>>,
        blocks: Option<Arc<crate::matrix::FederationBlocks>>,
    ) -> Self {
        let hits = Arc::new(AtomicUsize::new(0));
        let upstream = echo_upstream(hits.clone()).await;
        let public = PublicUrls {
            scheme: "http".into(),
            base_domain: CORE.into(),
            port: PORT,
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
        let profile = OwnerStore::for_test(dir.path(), &owner());
        let lifecycle = LifecycleConfig {
            public: public.clone(),
            data_dir: dir.path().to_owned(),
            apps_dir: dir.path().join("apps"),
            shares_dir: dir.path().join("shares"),
            relay_dir: dir.path().join("relay"),
            core_container: None,
            tz: "UTC".into(),
            puid: 1000,
            pgid: 1000,
            install_timeout: Duration::from_secs(5),
            owner: "owner".into(),
            profile: profile.clone(),
            gateway_port: 8080,
            plugin_slice: None,
            instance: Default::default(),
        };
        let manager = Arc::new(PluginManager::new(
            Engine::unreachable_for_test(),
            Store::open(dir.path().join("plugins")).unwrap(),
            http.clone(),
            hydra.as_ref().map(|(h, _)| h.clone()),
            lifecycle,
            ImageWorker::in_process(),
        ));
        let manifest =
            PluginPackage::manifest_of(WHOAMI, &Languages::from_header(Some("zh-Hans"))).unwrap();
        manager.register_for_test(manifest, PluginStatus::Running, Some(upstream.to_string()));
        let (tunnel, members) = loopback_tunnel_with(dir.path(), lan).await;
        let gateway = Gateway::new(
            public,
            profile,
            TOKEN.into(),
            members.clone(),
            manager.clone(),
            hydra,
            http,
            tunnel.clone(),
        )
        .with_matrix(matrix)
        .with_friends(friends)
        .with_federation_blocks(blocks);
        let app = router(Arc::new(gateway));
        let entry = TunnelEntry::spawn(&tunnel, app.clone());
        Self {
            app,
            plugins: manager,
            tunnel,
            members,
            hits,
            _entry: entry,
            _dir: dir,
        }
    }

    /// 主人经管理 API 登记或吊销设备（在进程内直接调用网关）。
    pub(super) async fn manage(&self, method: Method, path: &str, body: &str) -> StatusCode {
        let request = hyper::Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, format!("{CORE}:{PORT}"))
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap();
        self.app.clone().oneshot(request).await.unwrap().status()
    }

    async fn register(&self, device: &SecretKey) {
        let body = format!("{{\"endpoint_id\": \"{}\"}}", device.public());
        let status = self
            .manage(Method::POST, "/_wonton/api/devices", &body)
            .await;
        assert_eq!(status, StatusCode::CREATED);
    }
}

/// 测试里的手机：隧道客户端加本地代理，代理只接受带本次会话凭据的请求。
pub(super) struct Phone {
    client: TunnelClient,
    proxy: LocalProxy,
    authorization: String,
    http: HttpClient,
}

impl Phone {
    pub(super) async fn start(key: SecretKey, home: &Home) -> Self {
        Self::connect(key, &home.tunnel).await
    }

    pub(super) async fn connect(key: SecretKey, tunnel: &Tunnel) -> Self {
        let client = TunnelClient::new(loopback_endpoint(key).await, tunnel.endpoint.addr());
        let credential = ProxyCredential::generate();
        let pair = format!("{}:{}", credential.username(), credential.password());
        let authorization = format!("Basic {}", STANDARD.encode(pair));
        let access = ProxyAccess::Credential {
            credential,
            domain: CORE.into(),
        };
        let proxy = LocalProxy::bind(client.clone(), (Ipv4Addr::LOCALHOST, 0).into(), access)
            .await
            .unwrap();
        Self {
            client,
            proxy,
            authorization,
            http: HttpClient::new(),
        }
    }

    /// 等到隧道客户端看到服务端以“未授权”断开连接。
    pub(super) async fn wait_until_rejected(&self) {
        let mut status = self.client.watch_status();
        let rejected = TunnelStatus::Disconnected(TunnelError::Unauthorized);
        tokio::time::timeout(
            Duration::from_secs(15),
            status.wait_for(|status| *status == rejected),
        )
        .await
        .expect("等待服务端断开连接超时")
        .unwrap();
    }

    /// 经本地代理发请求的开头：`Host` 原样带到网关。
    pub(super) fn request(
        &self,
        method: Method,
        host: &str,
        path: &str,
    ) -> hyper::http::request::Builder {
        hyper::Request::builder()
            .method(method)
            .uri(format!("http://{}{path}", self.proxy.local_addr()))
            .header(header::HOST, format!("{host}:{PORT}"))
            .header(header::PROXY_AUTHORIZATION, &self.authorization)
    }

    pub(super) fn http(&self) -> HttpClient {
        self.http.clone()
    }

    /// 经本地代理打开一个页面。
    pub(super) async fn get(&self, host: &str, path: &str) -> (StatusCode, HeaderMap, String) {
        self.send(Method::GET, host, path, &[("accept", "text/html")])
            .await
    }

    /// 经本地代理 POST 一段 JSON。
    pub(super) async fn post_json(
        &self,
        host: &str,
        path: &str,
        body: &str,
    ) -> (StatusCode, String) {
        let headers = [("content-type", "application/json")];
        let (status, _, text) = self
            .send_with_body(Method::POST, host, path, &headers, body)
            .await;
        (status, text)
    }

    /// 经本地代理发一个请求，`Host` 原样带到网关。每个请求一条新连接，测试之间不受连接池影响。
    pub(super) async fn send(
        &self,
        method: Method,
        host: &str,
        path: &str,
        headers: &[(&str, &str)],
    ) -> (StatusCode, HeaderMap, String) {
        self.send_with_body(method, host, path, headers, "").await
    }

    async fn send_with_body(
        &self,
        method: Method,
        host: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> (StatusCode, HeaderMap, String) {
        let mut builder = hyper::Request::builder()
            .method(method)
            .uri(format!("http://{}{path}", self.proxy.local_addr()))
            .header(header::HOST, format!("{host}:{PORT}"))
            .header(header::PROXY_AUTHORIZATION, &self.authorization)
            .header(header::CONNECTION, "close");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let request = builder.body(Body::from(body.to_owned())).unwrap();
        let response = self.http.send(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }
}

pub(super) fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).map(|v| v.to_str().unwrap())
}

#[tokio::test]
async fn paired_device_opens_plugins_without_any_gateway_session() {
    let home = Home::start(None).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;

    let (status, _, text) = phone.get(PLUGIN, "/hello?x=1").await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let seen: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(seen["path"], "/hello?x=1");
    assert_eq!(seen["headers"]["host"], format!("{PLUGIN}:{PORT}"));
    assert!(seen["headers"].get("cookie").is_none(), "{seen}");
    assert!(
        seen["headers"].get("x-forwarded-for").is_none(),
        "隧道请求没有客户端 IP：{seen}"
    );
    assert_eq!(home.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn paired_devices_open_proxy_sign_in_pages_without_a_password() {
    let home = Home::start(None).await;
    let hits = register_vault(&home.plugins).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;
    let forged = [("cookie", "vault_web=forged")];
    let (status, headers, text) = phone.send(Method::GET, VAULT_HOST, "/leak", &forged).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(received(&text, "cookie"), [format!("vault_web={KEY}")]);
    assert!(leaked(&headers).is_empty(), "{headers:?}");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cross_plugin_requests_do_not_carry_the_device_identity() {
    let home = Home::start(None).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;

    // 隧道身份和 Cookie 一样随请求自动带上，所以按同一套规则只认本插件页面的请求和导航。
    let origin = |host: &str| format!("http://{host}:{PORT}");
    let cases = browser_cases(&origin(PLUGIN), &origin(OTHER));
    for case in &cases {
        let (status, headers, _) = phone
            .send(case.method.clone(), PLUGIN, "/notes", &case.headers())
            .await;
        case.check(status, &headers);
    }
    let recognized = cases.iter().filter(|c| c.recognized).count();
    assert_eq!(
        home.hits.load(Ordering::SeqCst),
        recognized,
        "不认人的请求不得到达插件"
    );
}

#[tokio::test]
async fn only_registered_devices_get_in_and_revocation_takes_effect_at_once() {
    let home = Home::start(None).await;
    let device = SecretKey::generate();
    let phone = Phone::start(device.clone(), &home).await;

    let (status, headers, _) = phone.get(PLUGIN, "/").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        header(&headers, wonton_tunnel::ERROR_HEADER),
        Some("unauthorized")
    );

    home.register(&device).await;
    // 被拒绝过的连接不会自己重拨，要有人重试。
    phone.client.retry_now();
    assert_eq!(phone.get(PLUGIN, "/").await.0, StatusCode::OK);

    let path = format!("/_wonton/api/devices/{}", device.public());
    assert_eq!(home.manage(Method::DELETE, &path, "").await, StatusCode::OK);
    // 吊销会立即断开这台设备现有的隧道连接，正在进行的请求随之中断。
    phone.wait_until_rejected().await;
    let (status, headers, _) = phone.get(PLUGIN, "/").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        header(&headers, wonton_tunnel::ERROR_HEADER),
        Some("unauthorized")
    );
    assert_eq!(
        home.hits.load(Ordering::SeqCst),
        1,
        "吊销后的请求不得到达插件"
    );
}

#[tokio::test]
async fn device_identity_signs_in_to_oidc_without_a_core_session() {
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let hydra = fake_hydra(accepted.clone()).await;
    let home = Home::start(Some(hydra)).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;

    let (status, headers, _) = phone
        .get(CORE, "/_wonton/oidc/login?login_challenge=abc")
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        header(&headers, "location"),
        Some("http://auth.wonton.localhost/after-login")
    );
    let (status, _, _) = phone
        .get(CORE, "/_wonton/oidc/consent?consent_challenge=def")
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let accepted = accepted.lock().unwrap();
    assert_eq!(accepted[0]["subject"], "owner");
    assert_eq!(
        accepted[1]["session"]["id_token"]["preferred_username"],
        "owner"
    );
}

/// 设备管理按人（ADR-0045）：主人的设备能列所有设备，登记设备仍只认主人令牌；家人的设备只能看到自己的
/// （家人一侧在 family_tests 里）。
#[tokio::test]
async fn the_owners_device_lists_devices_but_cannot_add_without_the_token() {
    let home = Home::start(None).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;

    let (status, _, _) = phone.get(CORE, "/_wonton/api/devices").await;
    assert_eq!(status, StatusCode::OK, "主人的设备看得到设备列表");
    let (status, headers, _) = phone
        .send(
            Method::POST,
            CORE,
            "/_wonton/api/devices",
            &[("content-type", "application/json")],
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "登记设备仍只认主人令牌");
    assert_eq!(header(&headers, "x-wonton-error"), Some("unauthenticated"));
}

/// ADR-0017：App 经隧道管理插件；插件网页的请求同样带着设备身份，读也要拒绝。
#[tokio::test]
async fn paired_devices_manage_plugins_but_web_pages_on_them_cannot() {
    let home = Home::start(None).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;
    let path = "/_wonton/api/plugins";

    let (status, _, text) = phone.send(Method::GET, CORE, path, &[]).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(
        serde_json::from_str::<wonton_protocol::InstalledPlugins>(&text).is_ok(),
        "{text}"
    );
    let same_origin = format!("http://{CORE}:{PORT}");
    for page in [("origin", same_origin.as_str()), ("sec-fetch-site", "none")] {
        let (status, _, text) = phone.send(Method::GET, CORE, path, &[page]).await;
        assert_eq!(status, StatusCode::OK, "{page:?}: {text}");
    }

    // 出网代理的设置页和网页局域网端口的开关都在 App 里（ADR-0023）。没带请求体，过了鉴权才回 400。
    let (status, _, text) = phone
        .send(Method::GET, CORE, "/_wonton/api/egress-proxy", &[])
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    for route in [
        "/_wonton/api/egress-proxy",
        "/_wonton/api/plugins/app.whoami/lan-webui",
    ] {
        let (status, _, text) = phone.send(Method::PUT, CORE, route, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{route}: {text}");
    }
    // 升级也在 App 里。过了鉴权才看请求：空的请求体不是合法的上传包，预览时这个插件没装。
    let upgrade = "/_wonton/api/plugins/app.whoami/upgrade";
    let (status, _, text) = phone.send(Method::POST, CORE, upgrade, &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    let preview = "/_wonton/api/plugins/app.whoami/upgrade/preview";
    let (status, headers, text) = phone.send(Method::POST, CORE, preview, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{text}");
    assert_eq!(header(&headers, "x-wonton-error"), Some("not_installed"));

    // 升级也在 App 里（ADR-0025）。这个核心没接上升级，过了鉴权才回 503。
    let (status, _, text) = phone
        .send(Method::GET, CORE, "/_wonton/api/core/upgrade", &[])
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{text}");

    let plugin_page = format!("http://{PLUGIN}:{PORT}");
    for (method, route) in [
        (Method::GET, path),
        (Method::POST, path),
        (Method::POST, "/_wonton/api/plugins/preview"),
        (Method::GET, "/_wonton/api/plugins/app.whoami"),
        (Method::DELETE, "/_wonton/api/plugins/app.whoami"),
        (Method::GET, "/_wonton/api/images/abc/icon-192.png"),
        (Method::GET, "/_wonton/api/store"),
        (
            Method::GET,
            "/_wonton/api/store/plugin?source=github:owner/repo",
        ),
        (Method::POST, "/_wonton/api/store/sources"),
        (Method::DELETE, "/_wonton/api/store/sources?address=x"),
        (Method::GET, "/_wonton/api/egress-proxy"),
        (Method::PUT, "/_wonton/api/egress-proxy"),
        (Method::GET, "/_wonton/api/core/upgrade"),
        (Method::POST, "/_wonton/api/core/upgrade"),
        (Method::DELETE, "/_wonton/api/core/upgrade"),
        (Method::POST, "/_wonton/api/core/upgrade/check"),
        (Method::PUT, "/_wonton/api/plugins/app.whoami/lan-webui"),
        (Method::POST, "/_wonton/api/plugins/app.whoami/upgrade"),
        (
            Method::POST,
            "/_wonton/api/plugins/app.whoami/upgrade/preview",
        ),
    ] {
        for page in [
            ("origin", plugin_page.as_str()),
            ("origin", "null"),
            ("sec-fetch-site", "same-site"),
            ("sec-fetch-site", "cross-site"),
        ] {
            let (status, headers, _) = phone.send(method.clone(), CORE, route, &[page]).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {route} {page:?}");
            assert_eq!(header(&headers, "x-wonton-error"), Some("cross_site"));
        }
    }

    // 设备列表认主人的设备（ADR-0045）；家人的设备会拿到 403 permission_required（family_tests）。
    // 局域网设置在这里读不了：Home 的引擎连不上 Docker，LAN 探测如实报 503，鉴权在 family_tests 的表里。
    let (status, _, _) = phone
        .send(Method::GET, CORE, "/_wonton/api/devices", &[])
        .await;
    assert_eq!(status, StatusCode::OK, "主人的设备能读");
    // App 连上后跟着服务器的地址走，插件网页不能借设备身份探测。
    let route = "/_wonton/api/tunnel";
    let (status, _, text) = phone.send(Method::GET, CORE, route, &[]).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let page = ("origin", plugin_page.as_str());
    let (status, _, _) = phone.send(Method::GET, CORE, route, &[page]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _, _) = lan(&home, Method::GET, CORE, path).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "局域网请求没有设备身份");
}

#[tokio::test]
async fn tunnel_api_reports_the_server_address_to_the_owner() {
    let home = Home::start(None).await;
    let announced = SocketAddr::new("192.168.1.2".parse().unwrap(), home.tunnel.port);
    home.tunnel.endpoint.add_external_addr(announced).await;
    let request = hyper::Request::builder()
        .uri("/_wonton/api/tunnel")
        .header(header::HOST, format!("{CORE}:{PORT}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let response = home.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let info: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(info["endpoint_id"], home.tunnel.endpoint.id().to_string());
    assert_eq!(info["base_domain"], CORE);
    assert_eq!(info["direct_addrs"][0], announced.to_string(), "{info}");
    let bound = home.tunnel.endpoint.bound_sockets()[0].to_string();
    assert!(
        !info["direct_addrs"]
            .as_array()
            .unwrap()
            .contains(&Value::String(bound)),
        "{info}"
    );
}

/// 局域网（没有隧道身份）的请求，在进程内直接交给网关。
pub(super) async fn lan(
    home: &Home,
    method: Method,
    host: &str,
    path: &str,
) -> (StatusCode, HeaderMap, Value) {
    let request = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, format!("{host}:{PORT}"))
        .body(Body::empty())
        .unwrap();
    let response = home.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn matrix_login_and_client_api_are_only_for_paired_devices() {
    let home = Home::start(None).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;
    let matrix = format!("matrix.{CORE}");

    // 已配对设备：两个入口都到得了，核心还没接入 Tuwunel，如实报不可用。
    let (status, headers, _) = phone
        .send(Method::POST, CORE, "/_wonton/matrix/login-token", &[])
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        header(&headers, "x-wonton-error"),
        Some("matrix_unavailable")
    );
    let (status, headers, body) = phone
        .send(Method::GET, &matrix, "/_matrix/client/versions", &[])
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        header(&headers, "x-wonton-error"),
        Some("matrix_unavailable")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["errcode"],
        "M_UNKNOWN"
    );
    // 联邦和管理接口不转发。
    for path in [
        "/_matrix/federation/v1/version",
        "/_synapse/admin/v1/register",
    ] {
        let (status, _, body) = phone.send(Method::GET, &matrix, path, &[]).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        let errcode = serde_json::from_str::<Value>(&body).unwrap()["errcode"].clone();
        assert_eq!(errcode, "M_UNRECOGNIZED", "{path}");
    }
    // 插件网页借着隧道身份跨站要登录令牌：核心域名上的跨站修改拦截挡住它。
    let origin = format!("http://{PLUGIN}:{PORT}");
    let cross_site = [("sec-fetch-site", "same-site"), ("origin", origin.as_str())];
    let (status, headers, _) = phone
        .send(
            Method::POST,
            CORE,
            "/_wonton/matrix/login-token",
            &cross_site,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(header(&headers, "x-wonton-error"), Some("cross_site"));

    // 局域网：没有隧道身份，拿不到登录令牌，也进不了 Matrix（不用 401，免得 SDK 当成令牌失效）。
    let (status, headers, _) = lan(&home, Method::POST, CORE, "/_wonton/matrix/login-token").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(header(&headers, "x-wonton-error"), Some("unauthenticated"));
    let (status, _, body) = lan(&home, Method::GET, &matrix, "/_matrix/client/versions").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["errcode"], "M_FORBIDDEN");
}

#[tokio::test]
async fn paired_devices_get_a_login_token_and_reach_the_client_api() {
    let (fake, config) = crate::matrix::fake::start().await;
    let state = tempfile::tempdir().unwrap();
    let matrix = Matrix::connect(&config, state.path(), HttpClient::new())
        .await
        .unwrap();
    let home = Home::start_with(None, Some(Arc::new(matrix))).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;
    let matrix_host = format!("matrix.{CORE}");

    let (status, _, body) = phone
        .send(Method::POST, CORE, "/_wonton/matrix/login-token", &[])
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token: super::matrix::LoginToken = serde_json::from_str(&body).unwrap();
    assert_eq!(token.homeserver_url, format!("http://{matrix_host}:{PORT}"));
    assert_eq!(token.user_id, "@owner:test");
    assert_eq!(token.login_type, "org.matrix.login.jwt");
    assert_eq!(token.expires_in, 120);
    let claims = crate::matrix::issuer::tests::verify(&token.token, &fake.key);
    assert_eq!(claims["sub"], "owner", "令牌只能登录主人");

    // 客户端接口转给 Tuwunel；写法可疑的路径在网关这一层就挡住，到不了管理接口。
    let (status, _, body) = phone
        .send(Method::GET, &matrix_host, "/_matrix/client/versions", &[])
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(serde_json::from_str::<Value>(&body).unwrap()["versions"].is_array());
    for path in [
        "/_matrix/client/../../_synapse/admin/v1/register",
        "/_matrix/client/%2e%2e/%2e%2e/_synapse/admin/v1/register",
        "/_matrix/client/.%2e/.%2E/_synapse/admin/v1/register",
    ] {
        let (status, headers, _) = phone.send(Method::GET, &matrix_host, path, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
        assert_eq!(
            header(&headers, "x-wonton-error"),
            Some("invalid_input"),
            "{path}"
        );
    }
    // 会话 ID 里的 `/` 编成 %2F：按会话取密钥备份的请求原样转给 Tuwunel（这里由假 Tuwunel 回 404 并报出收到的路径）。
    let room_keys = "/_matrix/client/v3/room_keys/keys/!r:test/ab%2Fcd";
    let (status, headers, body) = phone.send(Method::GET, &matrix_host, room_keys, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(header(&headers, "x-wonton-error"), None, "不是网关挡下的");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["path"],
        room_keys
    );
}

/// 插件网络里的容器发来的请求：网关看到的来源地址是 `from`。
/// 和 [`from_plugin`] 一样，但不带 Authorization：matrix-sdk 恢复会话前取的 `versions` 就是这种请求。
pub(super) async fn from_plugin_without_token(
    home: &Home,
    from: &str,
    path: &str,
) -> (StatusCode, HeaderMap, Value) {
    let request = hyper::Request::builder()
        .uri(path)
        .header(header::HOST, "matrix.wonton.internal:8080")
        .body(Body::empty())
        .unwrap();
    let addr: SocketAddr = from.parse().unwrap();
    let mut request = request;
    request.extensions_mut().insert(ConnectInfo(addr));
    let response = home.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, body)
}

pub(super) async fn from_plugin(
    home: &Home,
    from: &str,
    path: &str,
) -> (StatusCode, HeaderMap, Value) {
    let mut request = hyper::Request::builder()
        .uri(path)
        .header(header::HOST, "matrix.wonton.internal:8080")
        .header(header::AUTHORIZATION, "Bearer bot-token")
        .body(Body::empty())
        .unwrap();
    let addr: SocketAddr = from.parse().unwrap();
    request.extensions_mut().insert(ConnectInfo(addr));
    let response = home.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, body)
}
