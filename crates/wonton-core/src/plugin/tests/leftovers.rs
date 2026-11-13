//! 安装途中核心重启：留下的容器和网络再装时清掉，同名但不属于本插件的不动、如实报错。
//!
//! 核心重启用丢掉安装任务来扮演：和进程退出一样不回滚，记录也还没写。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::super::docker::Engine;
use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::preinstall::Backoff;
use super::super::{PluginId, PluginManager, PluginStatus};
use super::manager::{manager_in_container, manager_in_instance, manager_with};
use super::package;

const NETWORK: &str = "wonton-app-test";
const VOLUME: &str = "wonton-app-test--data";

/// 测试期间不会到点重试。
const NEVER: Backoff = Backoff {
    first: Duration::from_secs(3600),
    max: Duration::from_secs(3600),
};

fn app() -> String {
    let services = "  web:\n    image: nginx\n    volumes: ['data:/data']";
    format!(
        "volumes:\n  data: {{}}\n{}",
        package(services, "  permissions: []")
    )
}

fn id() -> PluginId {
    PluginId::parse("app.test").unwrap()
}

/// 只有一个默认插件（`app.test`）的默认插件目录。
fn defaults(dir: &tempfile::TempDir) -> PathBuf {
    let defaults = dir.path().join("defaults");
    fs::create_dir_all(defaults.join("test")).unwrap();
    fs::write(defaults.join("test/docker-compose.yml"), app()).unwrap();
    defaults
}

fn degraded(status: &Option<PluginStatus>) -> bool {
    matches!(status, Some(PluginStatus::Degraded { .. }))
}

fn status(m: &PluginManager) -> Option<PluginStatus> {
    m.list()
        .into_iter()
        .find(|p| p.id == id())
        .map(|p| p.status)
}

/// 核心在 Docker 收到 `call` 时重启：这一步之前建的资源都留着。
async fn restart_during(
    docker: &FakeDocker,
    engine: &Engine,
    dir: &tempfile::TempDir,
    defaults: &Path,
    call: &str,
) {
    let held = docker.hold(call);
    let m = Arc::new(manager_in_container(engine.clone(), dir));
    let task = tokio::spawn({
        let (m, defaults) = (m.clone(), defaults.to_owned());
        async move { m.ensure_defaults(&defaults, NEVER).await }
    });
    held.reached().await;
    task.abort();
    task.await.unwrap_err();
    held.release();
}

#[tokio::test]
async fn a_core_restart_midway_through_an_install_is_recovered_by_the_next_start() {
    let web = container_name("app-test", "web");
    let start = format!("POST /containers/{web}/start");
    // 网络建好、核心接进去了；容器也建好、跑起来了。
    for call in ["POST /containers/create", start.as_str()] {
        let (docker, engine) = FakeDocker::start().await;
        let dir = tempfile::tempdir().unwrap();
        let defaults = defaults(&dir);
        restart_during(&docker, &engine, &dir, &defaults, call).await;
        assert_eq!(docker.networks(), [NETWORK], "{call}");
        assert!(!dir.path().join("plugins/app.test.json").exists());

        let m = manager_in_container(engine.clone(), &dir);
        m.restore().await.unwrap();
        // 重试也没用的话这里要等一小时。
        m.ensure_defaults(&defaults, NEVER).await.unwrap();
        assert_eq!(status(&m), Some(PluginStatus::Running), "{call}");
        assert_eq!(docker.networks(), [NETWORK], "{call}");
        assert_eq!(docker.containers(), std::slice::from_ref(&web), "{call}");
        assert!(dir.path().join("plugins/app.test.json").exists());
    }
}

#[tokio::test]
async fn clearing_leftovers_keeps_what_the_install_goes_on_using() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let defaults = defaults(&dir);
    let web = container_name("app-test", "web");
    let start = format!("POST /containers/{web}/start");
    restart_during(&docker, &engine, &dir, &defaults, &start).await;
    assert_eq!(docker.volumes(), [VOLUME]);

    let m = manager_in_container(engine, &dir);
    m.ensure_defaults(&defaults, NEVER).await.unwrap();
    assert_eq!(status(&m), Some(PluginStatus::Running));
    assert_eq!(docker.volumes(), [VOLUME]);
    let deleted = format!("DELETE /volumes/{VOLUME}");
    assert!(!docker.writes().contains(&deleted), "卷里可能有数据");
}

/// 同一个插件在别的核心（实例名不同）里跑着：标签对不上，不算残留。
#[tokio::test]
async fn resources_of_another_core_are_not_leftovers() {
    let (docker, engine) = FakeDocker::start().await;
    let (formal_dir, dev_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let formal = manager_with(engine.clone(), &formal_dir);
    formal.install(&app(), &[], super::langs()).await.unwrap();
    let before = docker.writes().len();
    let dev = manager_in_instance(engine, &dev_dir, "alice");
    dev.install(&app(), &[], super::langs()).await.unwrap();
    let web = container_name("app-test", "web");
    assert!(docker.containers().contains(&web));
    let writes = docker.writes().split_off(before);
    assert!(!writes.contains(&format!("DELETE /containers/{web}")));
    assert!(!writes.contains(&format!("DELETE /networks/{NETWORK}")));
}

/// 回滚没做完留下的：这次按新的计划重建。
#[tokio::test]
async fn containers_labeled_for_the_plugin_without_a_record_are_replaced() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let web = container_name("app-test", "web");
    docker.add_container(&web, "app.test");
    docker.add_network(NETWORK, "app.test", json!({"Internal": false}));
    let m = manager_with(engine, &dir);
    m.install(&app(), &[], super::langs()).await.unwrap();
    assert_eq!(status(&m), Some(PluginStatus::Running));
    // 这个插件不出网，残留的网络能出网：重建后按这次的算。
    assert_eq!(docker.network(NETWORK).unwrap()["Internal"], json!(true));
    assert!(docker.created(&web).is_some(), "容器要按这次的计划重新创建");
}

async fn network_is_refused(prepare: impl Fn(&FakeDocker), owner: Option<&str>) {
    let (docker, engine) = FakeDocker::start().await;
    prepare(&docker);
    let before = docker.network(NETWORK);
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine.clone(), &dir);

    let err = m.install(&app(), &[], super::langs()).await.unwrap_err();
    assert_eq!(err.kind(), "conflict", "{err}");
    let reason = err.to_string();
    assert!(reason.contains(NETWORK), "{reason}");
    assert!(owner.is_none_or(|o| reason.contains(o)), "{reason}");
    let failed = m
        .installed_plugin(
            &super::super::PluginId::parse("app.test").unwrap(),
            &super::langs(),
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(failed).unwrap()["install_failure"]["kind"],
        "conflict"
    );

    // 预装没人等着回答：标成异常让主人看到。重试的话这里要等一小时，要主人动手的事不重试。
    let m = manager_with(engine, &dir);
    m.ensure_defaults(&defaults(&dir), NEVER).await.unwrap();
    let Some(PluginStatus::Degraded { reason }) = status(&m) else {
        panic!("应当标为异常：{:?}", status(&m));
    };
    assert!(reason.contains(NETWORK), "{reason}");
    assert!(owner.is_none_or(|o| reason.contains(o)), "{reason}");

    assert_eq!(docker.networks(), [NETWORK]);
    assert_eq!(docker.network(NETWORK), before, "网络要原样留着");
    let writes = docker.writes();
    assert!(
        !writes
            .iter()
            .any(|w| w.starts_with("DELETE /networks") || w.starts_with("POST /networks/create")),
        "{writes:?}"
    );
    assert!(docker.containers().is_empty(), "{:?}", docker.containers());
    assert!(docker.volumes().is_empty(), "{:?}", docker.volumes());
}

#[tokio::test]
async fn a_network_created_by_the_owner_is_reported_and_left_alone() {
    let owner = |docker: &FakeDocker| docker.add_unlabeled_network(NETWORK);
    network_is_refused(owner, None).await;
}

/// `app.foo.bar` 和 `app.foo-bar` 这样的 ID 的 slug 相同，网络就同名。
#[tokio::test]
async fn a_network_of_another_plugin_is_reported_and_left_alone() {
    let other = |docker: &FakeDocker| docker.add_network(NETWORK, "app.other", json!({}));
    network_is_refused(other, Some("app.other")).await;
}

#[tokio::test]
async fn a_default_plugin_refused_for_a_volume_of_another_owner_is_shown_as_well() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    docker.add_volume(VOLUME, "app.other");
    let m = manager_with(engine, &dir);
    m.ensure_defaults(&defaults(&dir), NEVER).await.unwrap();
    let Some(PluginStatus::Degraded { reason }) = status(&m) else {
        panic!("应当标为异常：{:?}", status(&m));
    };
    assert!(reason.contains(VOLUME), "{reason}");
    assert_eq!(docker.volumes(), [VOLUME]);
}

/// 没设实例名的另一套核心在运行：它的插件和残留一模一样，只有它接在网络上这一点不同。什么都不删，也不重试。
#[tokio::test]
async fn a_plugin_of_another_running_core_is_not_taken_for_leftovers() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let web = container_name("app-test", "web");
    docker.add_container(&web, "app.test");
    docker.add_network(NETWORK, "app.test", json!({}));
    let endpoint = json!({"NetworkingConfig": {"EndpointsConfig": {NETWORK: {}}}});
    docker.add_stranger_container("another-core", endpoint);

    let m = manager_in_container(engine, &dir);
    // 重试的话这里要等一小时。
    m.ensure_defaults(&defaults(&dir), NEVER).await.unwrap();
    let Some(PluginStatus::Degraded { reason }) = status(&m) else {
        panic!("应当标为异常：{:?}", status(&m));
    };
    assert!(reason.contains("another-core"), "{reason}");
    assert!(reason.contains(NETWORK), "{reason}");
    assert_eq!(docker.containers(), ["another-core", web.as_str()]);
    assert_eq!(docker.networks(), [NETWORK]);
    assert!(docker.writes().is_empty(), "{:?}", docker.writes());
}

/// 清理的前提：有安装记录的插件走不到部署，它的容器和网络不算残留。运行中、异常和已停止的都一样。
#[tokio::test]
async fn an_installed_plugin_is_never_taken_for_leftovers() {
    for failed in [false, true] {
        let (docker, engine) = FakeDocker::start().await;
        let dir = tempfile::tempdir().unwrap();
        manager_with(engine.clone(), &dir)
            .install(&app(), &[], super::langs())
            .await
            .unwrap();
        let web = container_name("app-test", "web");
        if failed {
            docker.set_state(
                &web,
                json!({"Status": "restarting", "Running": true, "ExitCode": 1}),
            );
        }
        let m = manager_with(engine, &dir);
        m.restore().await.unwrap();
        assert_eq!(degraded(&status(&m)), failed);

        let before = docker.writes().len();
        let err = m.install(&app(), &[], super::langs()).await.unwrap_err();
        assert_eq!(err.kind(), "conflict", "{err}");
        let writes = docker.writes().split_off(before);
        assert!(
            !writes.iter().any(|w| w.starts_with("DELETE")),
            "{writes:?}"
        );
    }
}

#[tokio::test]
async fn a_stopped_plugin_is_never_taken_for_leftovers() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    manager_with(engine.clone(), &dir)
        .install(&app(), &[], super::langs())
        .await
        .unwrap();
    docker.set_state(
        &container_name("app-test", "web"),
        json!({"Status": "exited", "Running": false, "ExitCode": 1}),
    );
    let m = manager_with(engine, &dir);
    m.restore().await.unwrap();
    assert_eq!(status(&m), Some(PluginStatus::Stopped));

    let before = docker.writes().len();
    let err = m.install(&app(), &[], super::langs()).await.unwrap_err();
    assert_eq!(err.kind(), "conflict", "{err}");
    let writes = docker.writes().split_off(before);
    assert!(
        !writes.iter().any(|w| w.starts_with("DELETE")),
        "{writes:?}"
    );
}
