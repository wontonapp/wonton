//! 安装失败的报错：Docker 不可达、镜像拉不下来，且不留下东西。

use super::super::super::fake_docker::FakeDocker;
use super::super::super::{PluginId, PluginPackage};
use super::super::{example, langs};
use super::{manager, manager_with};

#[tokio::test]
async fn docker_outage_is_reported_and_leaves_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir);
    let err = m
        .install_package(example("whoami"), &[], langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "docker_unavailable", "{err}");
    assert!(m.list().is_empty());
    let whoami = example("whoami");
    let system = PluginPackage::with_files(
        &whoami.source.replace("kind: app", "kind: system"),
        whoami.files().clone(),
        &langs(),
    )
    .unwrap();
    assert_eq!(
        m.install_package(system, &[], langs())
            .await
            .unwrap_err()
            .kind(),
        "docker_unavailable"
    );
    let id = PluginId::parse("app.whoami").unwrap();
    assert_eq!(
        m.uninstall(&id, true, &langs()).await.unwrap_err().kind(),
        "not_installed"
    );
}

#[tokio::test]
async fn an_image_that_cannot_be_pulled_fails_the_install_with_dockers_reason() {
    let (docker, engine) = FakeDocker::start().await;
    docker.fail_pulls(Some("manifest unknown"));
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let err = m
        .install_package(example("whoami"), &[], langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "image_pull_failed", "{err}");
    let failed = m
        .installed_plugin(&example("whoami").manifest.id, &langs())
        .unwrap();
    assert_eq!(
        serde_json::to_value(failed).unwrap()["install_failure"]["kind"],
        "image_pull_failed"
    );
    assert!(docker.containers().is_empty(), "{:?}", docker.containers());
}
