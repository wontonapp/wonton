//! 安装时插件声明的权限必须逐项确认。

use super::super::super::InstallError;
use super::super::super::fake_docker::FakeDocker;
use super::super::super::names::container_name;
use super::super::env_of_container;
use super::super::{example, langs, package};
use super::{manager, manager_with};

#[tokio::test]
async fn every_declared_permission_must_be_confirmed() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(&dir);
    let share = package(
        "  web:\n    image: nginx\n    environment:\n      NOTIFY: ${WONTON_NOTIFY_URL}",
        "  permissions: ['storage.share.photos:read', 'notify']",
    );
    let photos = vec!["storage.share.photos:read".parse().unwrap()];
    let err = m.install(&share, &photos, langs()).await.unwrap_err();
    assert!(
        matches!(&err, InstallError::PermissionNotGranted(p) if p.contains("notify")),
        "{err}"
    );
    let extra = vec!["models".parse().unwrap()];
    assert!(matches!(
        m.install_package(example("whoami"), &extra, langs()).await,
        Err(InstallError::Invalid(_))
    ));
    // notify 核心能兑现（ADR-0047）：授权后能装，环境里有发通知的地址。
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let all = vec![
        "storage.share.photos:read".parse().unwrap(),
        "notify".parse().unwrap(),
    ];
    m.install(&share, &all, langs()).await.unwrap();
    assert_eq!(m.list().len(), 1);
    let env = env_of_container(&docker, &container_name("app-test", "web"));
    assert_eq!(
        env["NOTIFY"],
        format!("http://{}:8080/notices", crate::config::PLUGIN_NOTIFY_HOST)
    );
}
