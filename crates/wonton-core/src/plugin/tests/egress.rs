//! 出网：没有声明 `network.egress` 的插件网络是 Docker 的内部网络；核心在容器里时连宿主也隔开。

use axum::routing::get;
use axum::{Json, Router};
use hyper::StatusCode;
use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::store::{InstalledRecord, Store};
use super::super::{PluginId, PluginStatus};
use super::manager::{manager_in_container, manager_with, manager_with_oidc};
use super::package;
use crate::config::OidcProviderConfig;
use crate::http::HttpClient;
use crate::oidc::Hydra;

fn app(permissions: &str) -> String {
    package(
        "  web:\n    image: nginx",
        &format!("  permissions: [{permissions}]"),
    )
}

const ISOLATED: &str = "isolated";

/// 创建插件网络时的请求里：（内部网络，IPv4/IPv6 的网关模式）。
fn created(docker: &FakeDocker) -> (bool, Value, Value) {
    let body = docker.network("wonton-app-test").expect("插件网络已创建");
    let options = &body["Options"];
    (
        body["Internal"] == true,
        options["com.docker.network.bridge.gateway_mode_ipv4"].clone(),
        options["com.docker.network.bridge.gateway_mode_ipv6"].clone(),
    )
}

#[tokio::test]
async fn plugins_get_no_way_out_unless_they_declare_network_egress() {
    // 核心在容器里：内部网络，网桥在宿主上也没有地址，插件连宿主上的服务也访问不到；核心加入网络访问插件。
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    manager_in_container(engine, &dir)
        .install(&app(""), &[])
        .await
        .unwrap();
    assert_eq!(created(&docker), (true, json!(ISOLATED), json!(ISOLATED)));
    let connect = "POST /networks/wonton-app-test/connect".to_owned();
    assert!(docker.writes().contains(&connect), "{:?}", docker.writes());

    // 核心直接跑在宿主上：它要经网桥访问插件，只能是普通的内部网络。
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    manager_with(engine, &dir)
        .install(&app(""), &[])
        .await
        .unwrap();
    assert_eq!(created(&docker), (true, Value::Null, Value::Null));

    // 声明了 network.egress：普通桥接网络，能访问外网和局域网。
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let granted = vec!["network.egress".parse().unwrap()];
    manager_in_container(engine, &dir)
        .install(&app("network.egress"), &granted)
        .await
        .unwrap();
    assert_eq!(created(&docker), (false, Value::Null, Value::Null));
    // 核心以低于默认（0）的优先级接入：默认路由和发布的端口留在核心自己的网络上（#90）。
    let priority = docker.gateway_priority("wonton-app-test");
    assert!(priority.is_some_and(|p| p < 0), "{priority:?}");
}

#[tokio::test]
async fn a_plugin_network_that_can_reach_out_without_egress_is_reported_after_restart() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let id = PluginId::parse("app.test").unwrap();
    Store::open(dir.path().join("plugins"))
        .unwrap()
        .save(&InstalledRecord {
            id: id.clone(),
            version: "1.0.0".into(),
            source: app(""),
            files: Default::default(),
            granted_permissions: Vec::new(),
            oidc_client_secret: None,
            installed_at: 1,
            origin: None,
        })
        .unwrap();
    docker.add_container(&container_name("app-test", "web"), "app.test");
    // 核心升级前装的插件：网络是普通桥接网络，插件没声明 network.egress 也能出网。
    docker.add_network("wonton-app-test", "app.test", json!({"Internal": false}));
    let m = manager_in_container(engine.clone(), &dir);
    m.restore().await.unwrap();
    let status = m.list().remove(0).status;
    assert!(
        matches!(status, PluginStatus::Degraded { .. }),
        "{status:?}"
    );

    let isolated = json!({"Internal": true, "Options": {
        "com.docker.network.bridge.gateway_mode_ipv4": ISOLATED,
        "com.docker.network.bridge.gateway_mode_ipv6": ISOLATED,
    }});
    docker.add_network("wonton-app-test", "app.test", isolated);
    let m = manager_in_container(engine, &dir);
    m.restore().await.unwrap();
    assert_eq!(m.list().remove(0).status, PluginStatus::Running);
}

/// 在本机起一个 HTTP 服务，返回地址。
pub(super) async fn serve(app: Router) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// 只回答安装时用到的端点：发现文档、登记客户端。
async fn fake_hydra() -> Hydra {
    let endpoint = |path: &str| format!("http://auth.wonton.localhost:8080{path}");
    let discovery = json!({
        "issuer": endpoint(""),
        "authorization_endpoint": endpoint("/oauth2/auth"),
        "token_endpoint": endpoint("/oauth2/token"),
        "userinfo_endpoint": endpoint("/userinfo"),
    });
    let app = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(move || async move { Json(discovery) }),
        )
        .route(
            "/admin/clients/{id}",
            axum::routing::put(|| async { StatusCode::OK }),
        );
    let addr = serve(app).await;
    let config = OidcProviderConfig {
        admin_url: format!("http://{addr}"),
        public_url: format!("http://{addr}"),
    };
    Hydra::new(HttpClient::new(), &config)
}

#[tokio::test]
async fn on_the_host_oidc_plugins_reach_the_core_through_their_own_network_gateway() {
    // 假 Docker 把插件容器的地址报成 127.0.0.1，安装最后一步检查的网页就是这个本机服务。
    let web = serve(Router::new().route("/", get(|| async { "ok" }))).await;
    let compose = package(
        "  web:\n    image: grafana\n    environment:\n      ISSUER: ${WONTON_OIDC_ISSUER}",
        &format!(
            "  permissions: []\n  webui: {{service: web, port: {}, sso: oidc, \
             oidc: {{redirect_paths: [/login]}}}}",
            web.port()
        ),
    );
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with_oidc(engine, &dir, fake_hydra().await);
    m.install(&compose, &[]).await.unwrap();
    // 内部网络没有默认路由，host-gateway 走不通；经插件网络的网关（宿主上网桥的地址）访问核心。
    let created = docker.created(&container_name("app-test", "web")).unwrap();
    assert_eq!(
        created["HostConfig"]["ExtraHosts"],
        json!(["auth.wonton.localhost:172.30.0.1"])
    );
    assert_eq!(created["HostConfig"]["NetworkMode"], "wonton-app-test");
}
