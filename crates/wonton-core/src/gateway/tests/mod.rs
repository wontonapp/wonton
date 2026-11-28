//! 网关行为测试：走真实 TCP 连接。插件上游和 Hydra 管理接口的替身在 [`super::fakes`] 里。
//! 共用夹具在本模块，用例按行为分在子模块。

use std::net::SocketAddr;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use http_body_util::BodyExt;
use hyper::header::{self, HeaderMap};
use hyper::{Method, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use super::fakes::loopback_tunnel;
use super::fakes::{bind, echo_upstream, serve};
pub(super) use super::proxy_sso_tests;
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
    include_str!("../../../../../plugins/examples/whoami/docker-compose.yml");

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

/// 网关测试接上真的总线（确认卡片用测试替身），装卸门槛的卡片流程走它。
fn test_bus(
    manager: &Arc<PluginManager>,
    data_dir: &std::path::Path,
    members: &Arc<crate::members::Members>,
    roles: &Arc<crate::roles::Roles>,
) -> crate::mcp::Endpoint {
    let bus = crate::mcp::Bus::new(
        manager.clone(),
        Arc::new(crate::mcp::Confirmations::new(
            Arc::new(crate::mcp::test_support::Cards::default()),
            Duration::from_secs(60),
        )),
        Arc::new(crate::mcp::audit::Audit::new(data_dir.join("audit.jsonl"))),
    );
    bus.attach_people("wonton-test".to_owned(), members.clone(), roles.clone());
    let inbox = crate::inbox::Inbox::open(
        data_dir.join("inbox"),
        Arc::new(crate::inbox::SystemClock),
        Arc::new(crate::inbox::AdminSet::new("owner".into(), Vec::new())),
    )
    .unwrap();
    bus.attach_inbox(inbox);
    bus.endpoint()
}

impl Harness {
    pub async fn start(hydra_admin: Option<SocketAddr>) -> Self {
        Self::start_full(
            hydra_admin,
            Egress::default(),
            None,
            StoreConfig::default(),
            None,
        )
        .await
    }

    pub async fn start_with(hydra_admin: Option<SocketAddr>, egress: Egress) -> Self {
        Self::start_full(hydra_admin, egress, None, StoreConfig::default(), None).await
    }

    pub async fn start_with_store(hydra_admin: Option<SocketAddr>, store: StoreConfig) -> Self {
        Self::start_full(hydra_admin, Egress::default(), None, store, None).await
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
            None,
        )
        .await
    }

    pub async fn start_on_host(host: crate::plugin::gpu::HostPaths) -> Self {
        Self::start_full(
            None,
            Egress::default(),
            None,
            StoreConfig::default(),
            Some(host),
        )
        .await
    }

    async fn start_full(
        hydra_admin: Option<SocketAddr>,
        egress: Egress,
        upgrades: Option<Arc<crate::upgrade::service::Upgrades>>,
        store_config: StoreConfig,
        host: Option<crate::plugin::gpu::HostPaths>,
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
        let manager = PluginManager::new(
            engine,
            Store::open(dir.path().join("plugins")).unwrap(),
            http.clone(),
            hydra.as_ref().map(|(h, _)| h.clone()),
            lifecycle,
            ImageWorker::in_process(),
        )
        .with_store(store_config)
        .with_lan(super::lan_api_tests::lan())
        .with_egress(egress);
        let manager = Arc::new(match host {
            Some(host) => manager.with_host(host),
            None => manager,
        });
        let manifest =
            PluginPackage::manifest_of(WHOAMI, &Languages::from_header(Some("zh-Hans"))).unwrap();
        manager.register_for_test(manifest, PluginStatus::Running, Some(upstream.to_string()));
        let (tunnel, members) = loopback_tunnel(dir.path()).await;
        let roles = Arc::new(crate::roles::Roles::open(dir.path().join("roles.json")).unwrap());
        let mcp = test_bus(&manager, dir.path(), &members, &roles);
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
        .with_upgrades(upgrades)
        .with_mcp(Some(mcp));
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

mod api;
mod entry;
mod forwarding;
mod identity;
mod isolation;
mod login;
mod routing;
mod upgrades;
