//! 几套核心共用一台 Docker（插件开发环境，`WONTON_INSTANCE`）：同一个插件可以在几套核心里同时装，每套核心只动
//! 自己建的资源。

use super::super::PluginId;
use super::super::fake_docker::FakeDocker;
use super::manager::{manager_in_instance, manager_with};
use super::package;

/// 带命名卷的插件。
fn app() -> String {
    let services = "  web:\n    image: nginx\n    volumes: ['data:/data']";
    format!(
        "volumes:\n  data: {{}}\n{}",
        package(services, "  permissions: []")
    )
}

#[tokio::test]
async fn cores_sharing_a_docker_host_only_touch_their_own_plugins() {
    let (docker, engine) = FakeDocker::start().await;
    let (formal_dir, dev_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let formal = manager_with(engine.clone(), &formal_dir);
    let dev = manager_in_instance(engine, &dev_dir, "alice");
    let id = PluginId::parse("app.test").unwrap();
    formal.install(&app(), &[]).await.unwrap();
    dev.install(&app(), &[]).await.unwrap();
    assert_eq!(
        docker.containers(),
        ["wonton-alice--app-test--web", "wonton-app-test--web"]
    );
    assert_eq!(
        docker.networks(),
        ["wonton-alice--app-test", "wonton-app-test"]
    );
    assert_eq!(
        docker.volumes(),
        ["wonton-alice--app-test--data", "wonton-app-test--data"]
    );
    let labels = |container: &str| docker.created(container).unwrap()["Labels"].clone();
    assert_eq!(
        labels("wonton-alice--app-test--web")["wonton.instance"],
        "alice"
    );
    assert!(
        labels("wonton-app-test--web")
            .get("wonton.instance")
            .is_none()
    );

    // 都带着 wonton.plugin=app.test，删数据卸载也只删自己那份。
    let report = dev.uninstall(&id, true).await.unwrap();
    assert_eq!(report.containers, ["wonton-alice--app-test--web"]);
    assert_eq!(report.volumes, ["wonton-alice--app-test--data"]);
    assert_eq!(docker.containers(), ["wonton-app-test--web"]);
    assert_eq!(docker.networks(), ["wonton-app-test"]);
    assert_eq!(docker.volumes(), ["wonton-app-test--data"]);

    dev.install(&app(), &[]).await.unwrap();
    let report = formal.uninstall(&id, true).await.unwrap();
    assert_eq!(report.containers, ["wonton-app-test--web"]);
    assert_eq!(report.volumes, ["wonton-app-test--data"]);
    assert_eq!(docker.containers(), ["wonton-alice--app-test--web"]);
    assert_eq!(docker.volumes(), ["wonton-alice--app-test--data"]);
}
