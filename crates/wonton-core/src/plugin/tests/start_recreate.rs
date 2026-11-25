//! 容器不在（被删了、Docker 数据丢了）时，启动按已安装记录照安装的路径重建容器再启动；网络不在也
//! 重建。做不到时按原因报类别：镜像拉不到是 `image_pull_failed`，记录读不懂是 `record_unusable`。

use std::sync::Arc;
use std::time::Duration;

use super::super::egress::settings::Egress;
use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::{PluginPackage, PluginStatus};
use super::lan::add_default_bridge;
use super::manager::{manager_in_container, manager_with};
use super::package;

fn compose() -> String {
    package("  app:\n    image: nginx\n", "  permissions: []")
}

fn app() -> String {
    container_name("app-test", "app")
}

/// 插件的网络：单实例核心里是 `wonton-<slug>`。
fn network() -> String {
    "wonton-app-test".to_owned()
}

#[tokio::test]
async fn deleted_containers_are_rebuilt_when_started() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine.clone(), &dir));
    let installed = manager
        .install(&compose(), &[], super::langs())
        .await
        .unwrap();
    assert_eq!(docker.creations(&app()), 1);

    // 开发栈 down/up、手动 docker rm 之后：容器不在了，记录还在。
    engine.remove_container(&app()).await.unwrap();
    assert!(!docker.containers().contains(&app()));

    let started = manager.start_detached(installed.id.clone()).await.unwrap();
    assert_eq!(started.status, PluginStatus::Running);
    assert_eq!(docker.creations(&app()), 2, "容器要照安装的路径重建");
    assert!(docker.containers().contains(&app()));
    // 再启动一次：容器在，不再重建。
    manager.start_detached(installed.id).await.unwrap();
    assert_eq!(docker.creations(&app()), 2);
}

#[tokio::test]
async fn a_deleted_network_is_rebuilt_when_started() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine.clone(), &dir));
    let installed = manager
        .install(&compose(), &[], super::langs())
        .await
        .unwrap();

    // 和容器一起被删掉（开发栈 down 会删网络）。
    engine.remove_container(&app()).await.unwrap();
    engine.remove_network(&network()).await.unwrap();

    let started = manager.start_detached(installed.id).await.unwrap();
    assert_eq!(started.status, PluginStatus::Running);
    assert!(
        docker.networks().contains(&network()),
        "{:?}",
        docker.networks()
    );
    assert_eq!(docker.creations(&app()), 2);
}

/// 出网插件的网络重建后，出口容器要接回去（安装时就是它建的端点），不然插件起来了也出不了网。
#[tokio::test]
async fn a_rebuilt_egress_network_gets_the_relay_back() {
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    docker.add_image_container("wonton-core", "wonton/core:test");
    let dir = tempfile::tempdir().unwrap();
    let manager =
        Arc::new(manager_in_container(engine.clone(), &dir).with_egress(Egress::default()));
    manager
        .store
        .save_egress_settings(&super::super::egress::settings::StoredEgress {
            url: Some("http://proxy.lan:1".to_owned()),
            ..Default::default()
        })
        .unwrap();
    let compose = package(
        "  web:\n    image: nginx\n",
        "  permissions: [network.egress]",
    );
    let grants = PluginPackage::parse(&compose, &super::langs())
        .unwrap()
        .manifest
        .permissions;
    let installed = manager
        .install(&compose, &grants, super::langs())
        .await
        .unwrap();
    let relay = "wonton-egress";
    let web = container_name("app-test", "web");
    assert!(
        docker.endpoint(&network(), relay).is_some(),
        "安装时出口容器接在插件网络上"
    );

    // 开发栈 down 的形态：核心、出口容器、插件容器都从网络上下来，网络才删得掉（有端点挂着时
    // Docker 不让删网络）。
    engine
        .disconnect_network(&network(), "wonton-core")
        .await
        .unwrap();
    engine
        .disconnect_network(&network(), "wonton-egress")
        .await
        .unwrap();
    engine.remove_container(&web).await.unwrap();
    for name in ["wonton-core", "wonton-egress", &web] {
        assert!(
            docker.endpoint(&network(), name).is_none(),
            "{name} 还挂在网络上"
        );
    }
    // 安装时验过代理的一次性容器可能还留着（down 也会把它清掉）。
    let _ = engine.remove_container("wonton-egress-check").await;
    engine.remove_network(&network()).await.unwrap();

    let started = manager.start_detached(installed.id).await.unwrap();
    assert_eq!(started.status, PluginStatus::Running);
    assert!(
        docker.endpoint(&network(), relay).is_some(),
        "重建的网络要接回出口容器"
    );
}

#[tokio::test]
async fn one_deleted_container_rebuilds_the_whole_plugin() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine.clone(), &dir));
    let installed = manager
        .install(
            &package(
                "  db:\n    image: nginx\n  app:\n    image: nginx\n    depends_on:\n      db: {condition: service_started}\n",
                "  permissions: []",
            ),
            &[],
            super::langs(),
        )
        .await
        .unwrap();
    let db = container_name("app-test", "db");
    engine.remove_container(&app()).await.unwrap();

    let started = manager.start_detached(installed.id).await.unwrap();
    assert_eq!(started.status, PluginStatus::Running);
    // 建容器成对出现才算整份重建：留着的一个也是新渲染的那份。
    assert_eq!(
        docker.creations(&app()),
        2,
        "{:?}",
        docker.creations(&app())
    );
    assert_eq!(docker.creations(&db), 2);
}

#[tokio::test]
async fn an_unpullable_image_fails_the_start_as_image_pull_failed() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine.clone(), &dir));
    let installed = manager
        .install(&compose(), &[], super::langs())
        .await
        .unwrap();
    engine.remove_container(&app()).await.unwrap();

    // 镜像没了且拉不到：重建走不下去，按类别报，不是笼统的上游错误。
    docker.fail_pulls(Some("pull access denied"));
    let error = manager.start_detached(installed.id).await.unwrap_err();
    assert_eq!(error.kind(), "image_pull_failed", "{error}");
    assert!(error.to_string().contains("pull access denied"), "{error}");
    assert!(matches!(
        manager.list()[0].status,
        PluginStatus::Degraded { .. }
    ));
}

#[tokio::test]
async fn a_start_timeout_for_a_vanished_container_settles_and_can_be_retried() {
    let (docker, engine) = FakeDocker::start().await;
    let engine = engine
        .with_request_limit_for_test(Duration::from_secs(1))
        .with_slow_request_limit_for_test(Duration::from_secs(1));
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine.clone(), &dir));
    let installed = manager
        .install(&compose(), &[], super::langs())
        .await
        .unwrap();
    engine.stop_container(&app()).await.unwrap();

    let hold = docker.hold(&format!("POST /containers/{}/start", app()));
    let start = {
        let manager = Arc::clone(&manager);
        let id = installed.id.clone();
        tokio::spawn(async move { manager.start_detached(id).await })
    };
    hold.reached().await;
    // Docker 没按时回答，容器最后也不在（start 请求丢了，或者被别的东西删了）。
    docker.remove(&app());
    tokio::time::sleep(Duration::from_millis(1500)).await;
    hold.release();
    // 超时后确认容器不在：失败类别如实是超时，原因说清容器已经不在。
    let error = start.await.unwrap().unwrap_err();
    assert_eq!(error.kind(), "docker_timed_out", "{error}");
    assert!(error.to_string().contains(&app()), "{error}");
    assert!(docker.containers().is_empty());
    assert!(matches!(
        manager.list()[0].status,
        PluginStatus::Degraded { .. }
    ));
    let recovery = manager
        .lifecycle_status(&installed.id)
        .unwrap()
        .recovery
        .unwrap();
    assert_eq!(recovery.category, "docker_timed_out");

    // 恢复重试照已安装记录重建，一轮就好；好了以后不再动容器。
    super::rebuild_resume::retry_due(&manager, &installed.id);
    manager.watch_containers().await;
    assert_eq!(manager.list()[0].status, PluginStatus::Running);
    assert_eq!(docker.containers(), [app().as_str()]);
    assert_eq!(docker.creations(&app()), 2);
    assert_eq!(
        manager.lifecycle_status(&installed.id).unwrap().recovery,
        None
    );
    let writes = docker.writes();
    manager.watch_containers().await;
    assert_eq!(docker.writes(), writes);
}

#[tokio::test]
async fn an_install_start_timeout_for_a_vanished_container_rebuilds_it() {
    let (docker, engine) = FakeDocker::start().await;
    let engine = engine
        .with_request_limit_for_test(Duration::from_secs(1))
        .with_slow_request_limit_for_test(Duration::from_secs(1));
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine.clone(), &dir));

    let hold = docker.hold(&format!("POST /containers/{}/start", app()));
    let install = {
        let manager = Arc::clone(&manager);
        tokio::spawn(async move { manager.install(&compose(), &[], super::langs()).await })
    };
    hold.reached().await;
    docker.remove(&app());
    tokio::time::sleep(Duration::from_millis(1500)).await;
    hold.release();
    // 容器不在了：照同一份计划重建再启动，这次装成功，不是半小时不动。
    let view = install.await.unwrap().unwrap();
    assert_eq!(view.status, PluginStatus::Running);
    assert_eq!(docker.creations(&app()), 2, "超时后容器不在，重建了一次");
    assert_eq!(docker.containers(), [app().as_str()]);
    let plugin = manager
        .installed_plugin(&installed_id(&manager), &super::langs())
        .unwrap();
    assert!(plugin.install_failure.is_none());
}

fn installed_id(manager: &super::super::PluginManager) -> super::super::PluginId {
    manager.list()[0].id.clone()
}

#[tokio::test]
async fn a_second_start_timeout_with_the_container_gone_is_a_definite_failure() {
    let (docker, engine) = FakeDocker::start().await;
    let engine = engine
        .with_request_limit_for_test(Duration::from_secs(1))
        .with_slow_request_limit_for_test(Duration::from_secs(1));
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine.clone(), &dir));

    let first = docker.hold(&format!("POST /containers/{}/start", app()));
    let install = {
        let manager = Arc::clone(&manager);
        tokio::spawn(async move { manager.install(&compose(), &[], super::langs()).await })
    };
    tokio::time::timeout(Duration::from_secs(10), first.reached())
        .await
        .expect("第一次 start 没来");
    let second = docker.hold(&format!("POST /containers/{}/start", app()));
    docker.remove(&app());
    tokio::time::sleep(Duration::from_millis(1500)).await;
    first.release();
    // 第一次超时后照计划重建过，这次启动也超时，最后又不在。
    tokio::time::timeout(Duration::from_secs(10), second.reached())
        .await
        .expect("第二次 start 没来");
    docker.remove(&app());
    tokio::time::sleep(Duration::from_millis(1500)).await;
    second.release();

    let error = install.await.unwrap().unwrap_err();
    assert_eq!(error.kind(), "docker_timed_out", "{error}");
    assert!(error.to_string().contains(&app()), "{error}");
    let plugin = manager
        .installed_plugin(&installed_id(&manager), &super::langs())
        .unwrap();
    assert!(matches!(
        plugin.status,
        wonton_protocol::PluginState::Degraded { .. }
    ));
    assert_eq!(
        plugin.install_failure.as_ref().unwrap().kind,
        "docker_timed_out"
    );
    assert!(docker.containers().is_empty(), "重建出来的容器也丢了");
    let writes = docker.writes();
    manager.watch_containers().await;
    assert_eq!(docker.writes(), writes, "失败安装不进巡检");
    assert!(matches!(
        manager.list()[0].status,
        PluginStatus::Degraded { .. }
    ));
}

#[tokio::test]
async fn an_unreadable_record_fails_the_start_as_record_unusable() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    let installed = manager
        .install(&compose(), &[], super::langs())
        .await
        .unwrap();

    // 记录文件坏了（磁盘出错、写坏了半截）：启动做不了，类别指向重新安装。
    let record = dir
        .path()
        .join("plugins")
        .join(format!("{}.json", installed.id));
    std::fs::write(&record, "not json").unwrap();

    let error = manager.start_detached(installed.id).await.unwrap_err();
    assert_eq!(error.kind(), "record_unusable", "{error}");
    assert!(
        error.to_string().contains(&record.display().to_string()),
        "{error}"
    );
}
