//! 插件状态跟着容器走：核心跑着的时候容器好了、坏了，状态都如实跟上。

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use serde_json::{Value, json};

use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::{Health, PluginManager, PluginStatus, docker::Engine};
use super::egress::serve;
use super::manager::{manager_in_container, manager_in_instance, manager_with};
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
async fn a_missing_plugin_network_is_recreated_and_containers_are_redeployed() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let network = "wonton-app-test";
    let container = container_name("app-test", "app");
    let first = manager_with(engine.clone(), &dir);
    first
        .install(
            &package("  app:\n    image: nginx\n", "  permissions: []"),
            &[],
            super::langs(),
        )
        .await
        .unwrap();
    let before = docker.creations(&container);
    docker.remove(&container);
    docker.remove_network(network);
    let restarted = manager_with(engine, &dir);
    restarted.restore().await.unwrap();

    assert_eq!(restarted.list()[0].status, PluginStatus::Running);
    assert_eq!(docker.creations(&container), before + 1);
    assert!(docker.network(network).is_some());
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
async fn an_old_container_watch_cannot_stop_a_reinstalled_plugin() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    let source = app().await;
    let installed = manager.install(&source, &[], super::langs()).await.unwrap();
    let old_token = manager.watch_token_for_test(&installed.id).unwrap();

    manager
        .uninstall(&installed.id, false, &super::langs())
        .await
        .unwrap();
    let reinstalled = manager.install(&source, &[], super::langs()).await.unwrap();
    let new_token = manager.watch_token_for_test(&reinstalled.id).unwrap();
    assert!(!Arc::ptr_eq(&old_token, &new_token));

    manager.saw_unavailable_for_test(&reinstalled.id, old_token, PluginStatus::Stopped);
    assert_eq!(manager.list()[0].status, PluginStatus::Running);
}

/// busybox：本地从 ECR Public 拉；CI 经 `WONTON_TEST_IMAGES` 指到镜像仓库副本（wonton-test-images）。
fn busybox() -> String {
    wonton_test_images::image(
        "public.ecr.aws/docker/library/busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    )
}

// 真实 Docker：开发栈 down（容器和网络都没了）后核心重启，restore 自己把网络和容器重建，不要求重装。
#[tokio::test]
#[ignore = "需要 Docker，删掉插件的容器和网络后 restore 能自愈"]
async fn a_stack_restart_with_lost_network_and_containers_recovers_on_restore() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    let image = busybox();
    crate::test_docker::ensure_image(&engine, &image).await;
    let names = crate::test_docker::Names::new();
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_in_instance(engine.clone(), &dir, names.name());
    let source = package(
        &format!("  app:\n    image: {image}\n    command: ['sleep', 'infinity']\n"),
        "  permissions: []",
    );
    let installed = manager.install(&source, &[], super::langs()).await.unwrap();
    let slug = installed.id.slug();
    let network = names.instance().network(&slug);
    let container = container_name(&slug, "app");

    // 相当于 compose down：先断开并删容器（网络还挂着端点时 Docker 不让删网络），再删网络。Docker 的
    // 端点清理是异步的，删网络要等它收完端点。
    engine
        .disconnect_network(&network, &container)
        .await
        .unwrap();
    engine.remove_container(&container).await.unwrap();
    for _ in 0..20 {
        match engine.remove_network(&network).await {
            Ok(()) => break,
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
        }
    }
    assert!(engine.network_reach(&network).await.unwrap().is_none());

    let restarted = manager_in_instance(engine.clone(), &dir, names.name());
    restarted.restore().await.unwrap();
    let [view] = <[_; 1]>::try_from(restarted.list()).expect("只有一个插件");
    assert_eq!(view.status, PluginStatus::Running, "不用重装就该自己回来");
    assert!(engine.network_reach(&network).await.unwrap().is_some());
    let running = engine
        .inspect_container(&container)
        .await
        .unwrap()
        .and_then(|info| info.state)
        .and_then(|state| state.running);
    assert_eq!(running, Some(true));

    restarted
        .uninstall(&installed.id, false, &super::langs())
        .await
        .unwrap();
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
