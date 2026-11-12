//! 出网：没有声明 `network.egress` 的插件网络是 Docker 的内部网络；核心在容器里时连宿主也隔开。

use axum::routing::get;
use axum::{Json, Router};
use hyper::StatusCode;
use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::super::egress::{EgressVars, PROXY_URL};
use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::package::PluginPackage;
use super::super::render::{Deployment, InstallContext, dry_run_context, render};
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
            lan_webui: false,
            relays: Vec::new(),
            firewalled: false,
            proxied: false,
            id: id.clone(),
            version: "1.0.0".into(),
            source: app(""),
            files: Default::default(),
            limits: Default::default(),
            granted_permissions: Vec::new(),
            oidc_client_secret: None,
            installed_at: 1,
            origin: None,
            written_by: None,
            ask_every_update: false,
        })
        .unwrap();
    docker.add_container(&container_name("app-test", "web"), "app.test");
    docker.add_container("wonton-core", "app.other");
    let ipam = json!({"Config": [{
        "Subnet": "172.30.0.0/24", "IPRange": "172.30.0.0/25", "Gateway": "172.30.0.1",
    }]});
    // 网络是普通桥接网络，插件没声明 network.egress 也能出网。
    docker.add_network(
        "wonton-app-test",
        "app.test",
        json!({"Internal": false, "IPAM": ipam}),
    );
    let m = manager_in_container(engine.clone(), &dir);
    m.restore().await.unwrap();
    let status = m.list().remove(0).status;
    assert!(
        matches!(status, PluginStatus::Degraded { .. }),
        "{status:?}"
    );

    let isolated = json!({"Internal": true, "IPAM": ipam, "Options": {
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

fn with_proxy(package: &PluginPackage) -> (InstallContext, Deployment) {
    let mut ctx = dry_run_context(package, None);
    ctx.egress = Some(EgressVars {
        auth_host: "auth.wonton.localhost".to_owned(),
    });
    let plan = render(package, &ctx).unwrap();
    (ctx, plan)
}

fn service<'a>(plan: &'a Deployment, name: &str) -> &'a super::super::render::ContainerPlan {
    plan.containers.iter().find(|c| c.service == name).unwrap()
}

#[test]
fn with_a_proxy_every_networked_service_of_an_egress_plugin_gets_the_variables() {
    let compose = package(
        "  web:\n    image: nginx\n    environment:\n      SEARX_PROXY: ${WONTON_PROXY_URL}\n    \
         healthcheck:\n      test: wget -q http://127.0.0.1/\n  \
         db:\n    image: postgres\n    environment:\n      HTTP_PROXY: \"\"\n  \
         sealed:\n    image: busybox\n    network_mode: none\n    \
         healthcheck:\n      test: wget -q http://127.0.0.1/",
        "  permissions: [network.egress]",
    );
    let package = PluginPackage::parse(&compose).unwrap();
    let (_, plan) = with_proxy(&package);

    let web = super::env_of(service(&plan, "web"));
    assert!(web.contains(&format!("HTTP_PROXY={PROXY_URL}")), "{web:?}");
    assert!(web.contains(&format!("SEARX_PROXY={PROXY_URL}")), "{web:?}");
    let no_proxy = web.iter().find(|l| l.starts_with("NO_PROXY=")).unwrap();
    assert!(
        no_proxy.contains(",auth.wonton.localhost,db,sealed,web,"),
        "{no_proxy}"
    );
    let db = super::env_of(service(&plan, "db"));
    assert!(
        db.contains(&"HTTP_PROXY=".to_owned()),
        "compose 写的优先：{db:?}"
    );
    let test = |name| {
        service(&plan, name)
            .body
            .healthcheck
            .clone()
            .unwrap()
            .test
            .unwrap()
    };
    assert!(
        test("web")[1].starts_with("unset http_proxy"),
        "{:?}",
        test("web")
    );

    // 没有网络的服务不写，健康检查也不改。
    let sealed = super::env_of(service(&plan, "sealed"));
    assert!(sealed.iter().all(|l| !l.contains("PROXY")), "{sealed:?}");
    assert_eq!(test("sealed")[1], "wget -q http://127.0.0.1/");
}

#[test]
fn without_a_proxy_only_the_empty_url_variable_is_injected() {
    let compose = package(
        "  web:\n    image: nginx\n    environment:\n      SEARX_PROXY: ${WONTON_PROXY_URL}\n    \
         healthcheck:\n      test: wget -q http://127.0.0.1/",
        "  permissions: [network.egress]",
    );
    let package = PluginPackage::parse(&compose).unwrap();
    let plan = render(&package, &dry_run_context(&package, None)).unwrap();
    let web = service(&plan, "web");
    assert_eq!(super::env_of(web), ["SEARX_PROXY="]);
    let test = web.body.healthcheck.clone().unwrap().test.unwrap();
    assert_eq!(test[1], "wget -q http://127.0.0.1/");
}

#[test]
fn a_plugin_without_egress_cannot_reference_the_proxy_url() {
    let compose = package(
        "  web:\n    image: nginx\n    environment:\n      P: ${WONTON_PROXY_URL}",
        "  permissions: []",
    );
    let package = PluginPackage::parse(&compose).unwrap();
    let error = render(&package, &dry_run_context(&package, None)).unwrap_err();
    assert!(error.to_string().contains("WONTON_PROXY_URL"), "{error}");
}

#[tokio::test]
async fn image_healthchecks_are_rewritten_only_when_a_proxy_is_configured() {
    let compose = package(
        "  web:\n    image: nginx\n  sealed:\n    image: nginx\n    network_mode: none",
        "  permissions: [network.egress]",
    );
    let package = PluginPackage::parse(&compose).unwrap();
    let (docker, engine) = FakeDocker::start().await;
    docker.set_image_healthcheck(
        "nginx",
        json!({"Test": ["CMD", "wget", "http://127.0.0.1/"], "Interval": 5_000_000_000_i64}),
    );
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);

    let (ctx, mut plan) = with_proxy(&package);
    m.rewrite_image_healthchecks(&ctx, &mut plan).await.unwrap();
    let check = service(&plan, "web").body.healthcheck.clone().unwrap();
    assert_eq!(
        check.test.unwrap(),
        ["CMD", "wget", "-Y", "off", "http://127.0.0.1/"]
    );
    assert_eq!(check.interval, Some(5_000_000_000));
    assert_eq!(service(&plan, "sealed").body.healthcheck, None);

    let ctx = dry_run_context(&package, None);
    let mut plan = render(&package, &ctx).unwrap();
    m.rewrite_image_healthchecks(&ctx, &mut plan).await.unwrap();
    assert_eq!(service(&plan, "web").body.healthcheck, None);
}

/// 按假 Docker 分配的网段 172.30.0.0/24 留给核心的地址。
const CORE_ADDRESS: &str = "172.30.0.254";

#[tokio::test]
async fn proxy_plugins_get_the_core_address_and_others_do_not() {
    let web = serve(Router::new().route("/", get(|| async { "ok" }))).await;
    let compose = package(
        "  web:\n    image: nginx\n    environment:\n      TRUSTED: ${WONTON_CORE_ADDRESS}/32",
        &format!(
            "  permissions: []\n  webui: {{service: web, port: {}, sso: proxy, \
             proxy: {{headers: {{Remote-User: \"{{user}}\"}}}}}}",
            web.port()
        ),
    );
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    // 核心直接跑在宿主上（开发时）：是网关地址。
    let m = manager_with(engine.clone(), &dir);
    m.install(&compose, &[]).await.unwrap();
    let env = docker.env(&container_name("app-test", "web"));
    assert!(env.contains(&"TRUSTED=172.30.0.1/32".to_owned()), "{env:?}");

    // 核心在容器里：是留给它的地址。网络按新建法建好后才算得出。
    let package = PluginPackage::parse(&compose).unwrap();
    let m = manager_in_container(engine, &dir);
    let network = "wonton-app-test";
    let var = m
        .core_address_var(&package.manifest, network)
        .await
        .unwrap();
    assert_eq!(var.as_deref(), Some(CORE_ADDRESS));

    // 不是代登录的插件不注入，引用了就报错。
    let other = PluginPackage::parse(&app("")).unwrap();
    assert_eq!(
        m.core_address_var(&other.manifest, network).await.unwrap(),
        None
    );
    let compose = package_with_env_ref();
    let package = PluginPackage::parse(&compose).unwrap();
    let error = render(&package, &dry_run_context(&package, None)).unwrap_err();
    assert!(error.to_string().contains("WONTON_CORE_ADDRESS"), "{error}");
}

fn package_with_env_ref() -> String {
    package(
        "  web:\n    image: nginx\n    environment:\n      TRUSTED: ${WONTON_CORE_ADDRESS}",
        "  permissions: []",
    )
}

#[tokio::test]
async fn the_core_attaches_at_its_reserved_address_and_reconnects_when_it_is_wrong() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_in_container(engine.clone(), &dir);
    m.install(&app("network.egress"), &["network.egress".parse().unwrap()])
        .await
        .unwrap();
    let network = "wonton-app-test";
    let body = docker.network(network).unwrap();
    let config = &body["IPAM"]["Config"][0];
    assert_eq!(config["Subnet"], "172.30.0.0/24");
    assert_eq!(config["IPRange"], "172.30.0.0/25");
    let endpoint = docker.endpoint(network, "wonton-core").unwrap();
    assert_eq!(endpoint["IPAMConfig"]["IPv4Address"], CORE_ADDRESS);
    assert_eq!(endpoint["GwPriority"], -1);

    // 核心容器还接着，但地址不对（比如手工接过）：断开重接。
    docker.add_endpoint(
        network,
        "wonton-core",
        json!({"IPAMConfig": {"IPv4Address": "172.30.0.77"}}),
    );
    docker.add_container("wonton-core", "app.other");
    let m = manager_in_container(engine, &dir);
    m.restore().await.unwrap();
    let endpoint = docker.endpoint(network, "wonton-core").unwrap();
    assert_eq!(endpoint["IPAMConfig"]["IPv4Address"], CORE_ADDRESS);
}
