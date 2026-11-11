//! 容器不在（被删了、Docker 数据丢了）时，启动按已安装记录照安装的路径重建容器再启动；网络不在也
//! 重建。做不到时按原因报类别：镜像拉不到是 `image_pull_failed`，记录读不懂是 `record_unusable`。

use std::sync::Arc;

use super::super::PluginStatus;
use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::manager::manager_with;
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
