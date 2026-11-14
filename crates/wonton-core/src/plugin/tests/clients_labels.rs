//! 卸载保留的数据不应让另一个插件占掉原来的入口标签。

use super::manager::manager_with;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::manifest::PluginId;

#[tokio::test]
async fn retained_label_survives_uninstall_reinstall_and_blocks_collisions() {
    let (_, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    let first = PluginId::parse("app.first.photos").unwrap();
    let second = PluginId::parse("app.second.photos").unwrap();
    manager
        .install(
            &super::manager::app("app.first.photos"),
            &[],
            super::langs(),
        )
        .await
        .unwrap();
    assert_eq!(
        manager
            .store()
            .load(&first)
            .unwrap()
            .unwrap()
            .entry_label
            .as_deref(),
        Some("photos")
    );
    manager
        .uninstall(&first, false, &super::langs())
        .await
        .unwrap();
    assert!(manager.store().load(&first).unwrap().is_none());
    manager
        .install(
            &super::manager::app("app.second.photos"),
            &[],
            super::langs(),
        )
        .await
        .unwrap();
    assert_eq!(
        manager
            .store()
            .load(&second)
            .unwrap()
            .unwrap()
            .entry_label
            .as_deref(),
        Some("photos-2")
    );
    manager
        .install(
            &super::manager::app("app.first.photos"),
            &[],
            super::langs(),
        )
        .await
        .unwrap();
    assert_eq!(
        manager
            .store()
            .load(&first)
            .unwrap()
            .unwrap()
            .entry_label
            .as_deref(),
        Some("photos")
    );
    manager
        .uninstall(&first, true, &super::langs())
        .await
        .unwrap();
    let third = PluginId::parse("app.third.photos").unwrap();
    manager
        .install(
            &super::manager::app("app.third.photos"),
            &[],
            super::langs(),
        )
        .await
        .unwrap();
    assert_eq!(
        manager
            .store()
            .load(&third)
            .unwrap()
            .unwrap()
            .entry_label
            .as_deref(),
        Some("photos")
    );
}
