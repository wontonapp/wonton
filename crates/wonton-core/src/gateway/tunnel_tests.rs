//! 经隧道的网关行为测试：真实的 iroh 隧道（只走回环地址）、设备本地代理和网关。插件上游和 Hydra
//! 管理接口的替身与 `tests.rs` 共用。

use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use http_body_util::BodyExt;
use hyper::header::{self, HeaderMap};
use hyper::{Method, StatusCode};
use iroh::endpoint::presets;
use iroh::{Endpoint, SecretKey};
use serde_json::Value;
use tower::ServiceExt;
use wonton_tunnel::{LocalProxy, TunnelClient, TunnelError, TunnelStatus};

use super::tests::{CORE, PLUGIN, TOKEN, WHOAMI, echo_upstream, fake_hydra, owner};
use super::{Gateway, router};
use crate::config::{OidcProviderConfig, PublicUrls};
use crate::device::Devices;
use crate::http::HttpClient;
use crate::oidc::Hydra;
use crate::plugin::docker::Engine;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{PluginManager, PluginPackage, PluginStatus};
use crate::tunnel::{Tunnel, TunnelEntry};

/// 浏览器看到的端口：经隧道时不监听，只用来拼地址。
const PORT: u16 = 18480;

/// 只走回环地址的 iroh endpoint：不连中继，不做地址发现。
async fn loopback_endpoint(key: SecretKey) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap()
}

/// 测试用的隧道：空的设备登记表，加一个只走回环地址的 endpoint。
pub(super) async fn loopback_tunnel(dir: &Path) -> Tunnel {
    Tunnel {
        devices: Arc::new(Devices::open(dir.join("devices.json")).unwrap()),
        endpoint: loopback_endpoint(SecretKey::generate()).await,
    }
}

struct Home {
    app: Router,
    tunnel: Tunnel,
    hits: Arc<AtomicUsize>,
    _entry: TunnelEntry,
    _dir: tempfile::TempDir,
}

impl Home {
    async fn start(hydra_admin: Option<SocketAddr>) -> Self {
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
        let lifecycle = LifecycleConfig {
            public: public.clone(),
            apps_dir: dir.path().join("apps"),
            shares_dir: dir.path().join("shares"),
            core_container: None,
            tz: "UTC".into(),
            puid: 1000,
            pgid: 1000,
            install_timeout: Duration::from_secs(5),
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
        let tunnel = loopback_tunnel(dir.path()).await;
        let gateway = Gateway::new(
            public,
            owner(),
            TOKEN.into(),
            manager,
            hydra,
            http,
            tunnel.clone(),
        );
        let app = router(Arc::new(gateway));
        let entry = TunnelEntry::spawn(tunnel.endpoint.clone(), &tunnel.devices, app.clone());
        Self {
            app,
            tunnel,
            hits,
            _entry: entry,
            _dir: dir,
        }
    }

    /// 主人经管理 API 登记或吊销设备（在进程内直接调用网关）。
    async fn manage(&self, method: Method, path: &str, body: &str) -> StatusCode {
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

/// 测试里的手机：隧道客户端加本地代理。
struct Phone {
    client: TunnelClient,
    proxy: LocalProxy,
    http: HttpClient,
}

impl Phone {
    async fn start(key: SecretKey, home: &Home) -> Self {
        let client = TunnelClient::new(loopback_endpoint(key).await, home.tunnel.endpoint.addr());
        let proxy = LocalProxy::bind(client.clone(), (Ipv4Addr::LOCALHOST, 0).into())
            .await
            .unwrap();
        Self {
            client,
            proxy,
            http: HttpClient::new(),
        }
    }

    /// 等到隧道客户端看到服务端以“未授权”断开连接。
    async fn wait_until_rejected(&self) {
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

    /// 经本地代理发一个请求，`Host` 原样带到网关。每个请求一条新连接，测试之间不受连接池影响。
    async fn get(&self, host: &str, path: &str) -> (StatusCode, HeaderMap, String) {
        let request = hyper::Request::builder()
            .method(Method::GET)
            .uri(format!("http://{}{path}", self.proxy.local_addr()))
            .header(header::HOST, format!("{host}:{PORT}"))
            .header(header::ACCEPT, "text/html")
            .header(header::CONNECTION, "close")
            .body(Body::empty())
            .unwrap();
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

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
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

#[tokio::test]
async fn management_api_still_requires_the_owner_token() {
    let home = Home::start(None).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;

    for path in [
        "/_wonton/api/plugins",
        "/_wonton/api/devices",
        "/_wonton/api/tunnel",
    ] {
        let (status, headers, _) = phone.get(CORE, path).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(header(&headers, "x-wonton-error"), Some("unauthenticated"));
    }
}

#[tokio::test]
async fn tunnel_api_reports_the_server_address_to_the_owner() {
    let home = Home::start(None).await;
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
    let bound = home.tunnel.endpoint.bound_sockets()[0].to_string();
    assert!(
        info["direct_addrs"]
            .as_array()
            .unwrap()
            .contains(&Value::String(bound)),
        "{info}"
    );
}
