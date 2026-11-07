//! 真实 Docker：安装、停止、重启核心读取 Stopped、HTTP 启动、幂等与数据保留。
//! 这些测试验证行为不验证速度，期限和生产一致；共用 runner 忙时 Docker 起容器会慢。

use std::sync::Arc;

use bollard::models::ContainerStateStatusEnum;
use bollard::query_parameters::StopContainerOptionsBuilder;
use iroh::{Endpoint, SecretKey};
use serde_json::Value;
use tokio::net::TcpListener;

use super::super::docker::Engine;
use super::super::{PluginManager, PluginStatus};
use super::manager::manager_in_instance;
use super::package;
use crate::config::DEFAULT_INSTALL_TIMEOUT;
use crate::device::Devices;
use crate::gateway::{Gateway, router};
use crate::http::HttpClient;
use crate::test_docker::Names;
use crate::tunnel::Tunnel;

const IMAGE: &str = "public.ecr.aws/docker/library/busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec";
const TOKEN: &str = "stopped-integration-owner-token";

async fn gateway(
    manager: Arc<PluginManager>,
    dir: &std::path::Path,
) -> (u16, tokio::task::JoinHandle<()>) {
    let endpoint = Endpoint::builder(iroh::endpoint::presets::Minimal)
        .secret_key(SecretKey::generate())
        .clear_ip_transports()
        .bind_addr((std::net::Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let tunnel = Tunnel::new(
        Arc::new(Devices::open(dir.join("devices.json")).unwrap()),
        Arc::new(|| crate::plugin::languages::Languages::from_header(None)),
        endpoint,
        0,
        wonton_tunnel::relays::RelayTable::builtin_n0(),
        manager.lan_addresses(),
    );
    let app = router(Arc::new(Gateway::new(
        manager.cfg.public.clone(),
        manager.cfg.profile.clone(),
        TOKEN.to_owned(),
        manager,
        None,
        HttpClient::new(),
        tunnel,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (port, task)
}

fn compose() -> String {
    package(
        &format!(
            "  init:\n    image: {IMAGE}\n    restart: 'no'\n    volumes: ['${{WONTON_APP_DATA}}:/data']\n    command: ['sh', '-c', 'echo initialized >> /data/init-runs; echo persistent > /data/index.html']\n  app:\n    image: {IMAGE}\n    init: true\n    volumes: ['${{WONTON_APP_DATA}}:/data']\n    command: ['httpd', '-f', '-p', '8000', '-h', '/data']\n    depends_on:\n      init: {{condition: service_completed_successfully}}\n    healthcheck:\n      test: ['CMD', 'wget', '-q', '-O', '/dev/null', 'http://127.0.0.1:8000/']\n      interval: 1s\n      timeout: 1s\n      retries: 3\n"
        ),
        "  permissions: []\n  webui: {service: app, port: 8000, sso: none}",
    )
}

async fn exercise(
    manager: Arc<PluginManager>,
    engine: &Engine,
    dir: &tempfile::TempDir,
    names: &Names,
) {
    let installed = manager
        .install(&compose(), &[], super::langs())
        .await
        .unwrap();
    assert_eq!(installed.status, PluginStatus::Running);
    let app = names.container("app-test", "app");
    let init = names.container("app-test", "init");
    let app_before = engine.inspect_container(&app).await.unwrap().unwrap();
    let init_before = engine.inspect_container(&init).await.unwrap().unwrap();
    let data = dir.path().join("apps/app-test");
    let initialized = std::fs::read(data.join("init-runs")).unwrap();
    assert_eq!(initialized, b"initialized\n");

    bollard::Docker::connect_with_defaults()
        .unwrap()
        .stop_container(
            &app,
            Some(StopContainerOptionsBuilder::default().t(2).build()),
        )
        .await
        .unwrap();
    assert_eq!(
        engine
            .inspect_container(&app)
            .await
            .unwrap()
            .unwrap()
            .state
            .unwrap()
            .status,
        Some(ContainerStateStatusEnum::EXITED)
    );
    manager.watch_containers().await;
    assert_eq!(manager.list()[0].status, PluginStatus::Stopped);
    let mut restarted = manager_in_instance(engine.clone(), dir, names.name());
    restarted.cfg.install_timeout = DEFAULT_INSTALL_TIMEOUT;
    let restarted = Arc::new(restarted);
    restarted.restore().await.unwrap();
    assert_eq!(restarted.list()[0].status, PluginStatus::Stopped);

    let (port, server) = gateway(restarted.clone(), dir.path()).await;
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://127.0.0.1:{port}/_wonton/api/plugins/app.test/start");
    for _ in 0..2 {
        let response = http
            .post(&url)
            .header("host", "wonton.localhost:8080")
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        assert_eq!(status.as_u16(), 200, "{body}");
        assert_eq!(body["status"]["state"], "running");
    }
    let route = restarted.route("app-test").unwrap();
    let upstream = route.upstream.unwrap();
    let page = http
        .get(upstream.url("/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(page, "persistent\n");
    assert_eq!(restarted.list()[0].status, PluginStatus::Running);
    let app_after = engine.inspect_container(&app).await.unwrap().unwrap();
    let init_after = engine.inspect_container(&init).await.unwrap().unwrap();
    assert_eq!(app_before.id, app_after.id);
    assert_eq!(
        init_before.state.unwrap().started_at,
        init_after.state.unwrap().started_at
    );
    assert_eq!(std::fs::read(data.join("init-runs")).unwrap(), initialized);
    assert_eq!(
        app_after.state.unwrap().status,
        Some(ContainerStateStatusEnum::RUNNING)
    );
    println!(
        "{}: exited → Stopped（巡检和核心重启）→ POST start 200 → Running；重复 start 200；网页 persistent；容器 ID 和一次性服务未改变",
        names.name()
    );
    server.abort();
}

#[tokio::test]
#[ignore = "需要 Docker，真实容器 停→Stopped→start→Running"]
async fn stopped_plugin_starts_existing_containers_via_http() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    let names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    let mut manager = manager_in_instance(engine.clone(), &dir, names.name());
    manager.cfg.install_timeout = DEFAULT_INSTALL_TIMEOUT;
    exercise(Arc::new(manager), &engine, &dir, &names).await;
}
