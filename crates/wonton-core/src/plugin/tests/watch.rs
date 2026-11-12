//! 插件状态跟着容器走：核心跑着的时候容器好了、坏了，状态都如实跟上。

use axum::Router;
use axum::routing::get;
use serde_json::{Value, json};

use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::{Health, PluginManager, PluginStatus};
use super::egress::serve;
use super::manager::{manager_in_container, manager_with};
use super::package;

async fn app() -> String {
    let port = serve(Router::new().route("/", get(|| async { "ok" })))
        .await
        .port();
    package(
        "  app:\n    image: nginx\n",
        &format!("  webui: {{service: app, port: {port}, sso: none}}\n  permissions: []"),
    )
}

fn restarting() -> Value {
    json!({"Status": "restarting", "Running": true, "Restarting": true})
}

fn with_health(status: &str) -> Value {
    json!({"Status": "running", "Running": true, "Health": {"Status": status}})
}

fn seen(m: &PluginManager) -> (PluginStatus, Option<Health>) {
    let [view] = <[_; 1]>::try_from(m.list()).expect("只有一个插件");
    (view.status, view.health)
}

fn degraded(m: &PluginManager) -> bool {
    matches!(seen(m).0, PluginStatus::Degraded { .. })
}

#[tokio::test]
async fn a_container_restarting_at_startup_recovers_when_it_becomes_healthy() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let container = container_name("app-test", "app");
    let installed = manager_with(engine.clone(), &dir);
    installed
        .install(&app().await, &[], super::langs())
        .await
        .unwrap();

    docker.set_state(&container, restarting());
    let m = manager_with(engine, &dir);
    m.restore().await.unwrap();
    assert!(degraded(&m), "{:?}", seen(&m));
    // 路由留着（容器还在），只是网页服务还不能用，网关回 503。
    let route = m.route("app-test", "/").unwrap();
    assert!(route.upstream.is_some());
    assert!(!route.web_serving);

    m.watch_containers().await;
    assert!(degraded(&m), "还在重启，不算好了：{:?}", seen(&m));

    docker.set_state(&container, with_health("healthy"));
    m.watch_containers().await;
    assert_eq!(seen(&m), (PluginStatus::Running, Some(Health::Healthy)));
    assert!(
        m.route("app-test", "/").unwrap().upstream.is_some(),
        "容器好了，网页也要能打开"
    );
}

#[tokio::test]
async fn the_core_rejoins_the_network_of_a_plugin_that_crashes_without_it() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let network = "wonton-app-test";
    manager_in_container(engine.clone(), &dir)
        .install(
            &package("  app:\n    image: nginx\n", "  permissions: []"),
            &[],
            super::langs(),
        )
        .await
        .unwrap();

    // 核心容器重建后不在插件网络里，靠核心的内部别名才起得来的插件一直崩。
    engine
        .disconnect_network(network, "wonton-core")
        .await
        .unwrap();
    docker.add_container("wonton-core", "app.other");
    docker.set_state(&container_name("app-test", "app"), restarting());
    let m = manager_in_container(engine, &dir);
    m.restore().await.unwrap();

    assert!(degraded(&m), "{:?}", seen(&m));
    assert!(docker.endpoint(network, "wonton-core").is_some());
}

#[tokio::test]
async fn a_running_plugin_follows_its_container_down_and_back() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let container = container_name("app-test", "app");
    let m = manager_with(engine, &dir);
    m.install(&app().await, &[], super::langs()).await.unwrap();

    docker.set_state(&container, with_health("unhealthy"));
    m.watch_containers().await;
    let (PluginStatus::Degraded { reason }, None) = seen(&m) else {
        panic!("{:?}", seen(&m));
    };
    assert!(
        reason.contains(&super::message(
            "plugin-install-container-unhealthy-noprobe",
            &[("container", &container)],
        )),
        "{reason}"
    );

    docker.set_state(&container, restarting());
    m.watch_containers().await;
    let (PluginStatus::Degraded { reason }, None) = seen(&m) else {
        panic!("{:?}", seen(&m));
    };
    assert!(
        reason.contains(&super::message(
            "plugin-install-container-not-running",
            &[("container", &container), ("status", "RESTARTING")],
        )),
        "原因也跟着容器走：{reason}"
    );

    docker.set_state(&container, with_health("starting"));
    m.watch_containers().await;
    assert_eq!(seen(&m), (PluginStatus::Running, Some(Health::Unknown)));

    docker.set_state(&container, with_health("healthy"));
    m.watch_containers().await;
    assert_eq!(seen(&m), (PluginStatus::Running, Some(Health::Healthy)));
}

#[tokio::test]
async fn a_plugin_being_rebuilt_is_left_alone() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let view = m.install(&app().await, &[], super::langs()).await.unwrap();
    assert!(m.begin_rebuild(&view.id));

    let exited = json!({"Status": "exited", "Running": false, "ExitCode": 1});
    docker.set_state(&container_name("app-test", "app"), exited);
    m.watch_containers().await;
    assert_eq!(seen(&m).0, PluginStatus::Running);
}

#[tokio::test]
async fn trouble_that_is_not_about_containers_stays_until_someone_deals_with_it() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let view = m.install(&app().await, &[], super::langs()).await.unwrap();
    let status = PluginStatus::Degraded {
        reason: "卸载未完成".to_owned(),
    };
    m.set(&view.id, status.clone(), Some(None));

    m.watch_containers().await;
    assert_eq!(seen(&m).0, status);
}

/// 网页服务加一个辅助容器。
async fn web_and_worker() -> String {
    let port = serve(Router::new().route("/", get(|| async { "ok" })))
        .await
        .port();
    package(
        "  web:\n    image: nginx\n  worker:\n    image: redis\n",
        &format!("  webui: {{service: web, port: {port}}}\n  permissions: []"),
    )
}

// 辅助容器坏了：插件整体异常，原因里列出它和它最后一条探针；网页服务的容器好着，网关照常转发。
#[tokio::test]
async fn an_unhealthy_auxiliary_container_leaves_the_web_page_served() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    m.install(&web_and_worker().await, &[], super::langs())
        .await
        .unwrap();

    let worker = container_name("app-test", "worker");
    docker.set_state(
        &worker,
        json!({
            "Status": "running", "Running": true,
            "Health": {"Status": "unhealthy", "Log": [
                {"ExitCode": 0, "Output": "早先通过的一次"},
                {"ExitCode": 28, "Output": "curl: (28) Operation timed out"},
            ]},
        }),
    );
    m.watch_containers().await;

    let (PluginStatus::Degraded { reason }, None) = seen(&m) else {
        panic!("{:?}", seen(&m));
    };
    assert!(reason.contains(&worker), "{reason}");
    assert!(
        reason.contains("curl: (28) Operation timed out"),
        "{reason}"
    );
    assert!(!reason.contains("早先通过的一次"), "只要最后一条：{reason}");
    let route = m.route("app-test", "/").unwrap();
    assert!(route.web_serving, "{reason}");
    assert!(route.upstream.is_some());
}

// 网页服务的容器自己不健康：路由不算能转发，网关回 503。
#[tokio::test]
async fn an_unhealthy_web_container_stops_serving() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    m.install(&web_and_worker().await, &[], super::langs())
        .await
        .unwrap();

    docker.set_state(&container_name("app-test", "web"), with_health("unhealthy"));
    m.watch_containers().await;

    assert!(degraded(&m), "{:?}", seen(&m));
    let route = m.route("app-test", "/").unwrap();
    assert!(!route.web_serving);
    assert!(route.upstream.is_some());
}

/// MCP 服务加一个辅助容器。
fn mcp_and_worker() -> String {
    package(
        "  mcp:\n    image: example/mcp\n  worker:\n    image: redis\n",
        "  mcp: {service: mcp, port: 9000, transport: streamable-http}\n  permissions: []",
    )
}

// 辅助容器坏了，MCP 服务自己好着：总线照常拿着它。
#[tokio::test]
async fn an_unhealthy_auxiliary_container_leaves_the_mcp_service_alone() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    m.install(&mcp_and_worker(), &[], super::langs())
        .await
        .unwrap();
    assert_eq!(m.mcp_servers().len(), 1);

    docker.set_state(
        &container_name("app-test", "worker"),
        with_health("unhealthy"),
    );
    m.watch_containers().await;

    assert!(degraded(&m), "{:?}", seen(&m));
    assert_eq!(m.mcp_servers().len(), 1, "{:?}", seen(&m));

    // MCP 服务自己也坏了：不再交给总线。
    docker.set_state(&container_name("app-test", "mcp"), with_health("unhealthy"));
    m.watch_containers().await;
    assert!(m.mcp_servers().is_empty());
}
