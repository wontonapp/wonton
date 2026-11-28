//! 经隧道的网关行为测试：真实的 iroh 隧道（只走回环地址）、设备本地代理（和 App 一样带会话凭据）和
//! 网关。插件上游和 Hydra 管理接口的替身与 `tests` 共用。

use std::net::{Ipv4Addr, SocketAddr};
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
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

use super::fakes::{echo_upstream, loopback_endpoint, loopback_tunnel_relays};
use super::tests::{CORE, TOKEN, WHOAMI, owner};
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
use wonton_tunnel::relays::RelayTable;
use wonton_tunnel::{
    LocalProxy, ProxyAccess, ProxyCredential, TunnelClient, TunnelError, TunnelStatus,
};

mod devices;
mod matrix;
mod oidc;
mod plugin_api;
mod plugin_pages;
mod tunnel_api;

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
        Self::start_full(None, None, lan, None, None, RelayTable::builtin_n0()).await
    }

    /// 中继表由调用方给的核心。
    pub(super) async fn start_relays(relays: RelayTable) -> Self {
        Self::start_full(
            None,
            None,
            std::collections::BTreeSet::from(["192.168.1.2".parse().unwrap()]),
            None,
            None,
            relays,
        )
        .await
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
            RelayTable::builtin_n0(),
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
            RelayTable::builtin_n0(),
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
        relays: RelayTable,
    ) -> Self {
        let hits = Arc::new(AtomicUsize::new(0));
        let upstream = echo_upstream(hits.clone()).await;
        let public = PublicUrls {
            scheme: "http".into(),
            base_domain: CORE.into(),
            port: PORT,
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
        let (tunnel, members) = loopback_tunnel_relays(dir.path(), lan, relays).await;
        let roles = Arc::new(crate::roles::Roles::open(dir.path().join("roles.json")).unwrap());
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
        .with_roles(roles.clone())
        .with_friends(friends)
        .with_federation_blocks(blocks);
        let gateway = Arc::new(gateway);
        let app = router(gateway.clone());
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

    pub(super) async fn register(&self, device: &SecretKey) {
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
