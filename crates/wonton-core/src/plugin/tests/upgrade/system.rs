use super::*;

/// 没有来源的默认插件（Hermes 这样随核心预装的）：看官方索引。
#[tokio::test]
pub(crate) async fn a_bundled_default_plugin_is_offered_from_the_official_index() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index");
    let m = Arc::new(manager_with(engine, &dir).with_store(StoreConfig {
        index_dir: Some(index.clone()),
        community: false,
    }));
    let defaults = dir.path().join("defaults");
    fs::create_dir_all(defaults.join("hello")).unwrap();
    let v1 = hello("1.0.0", "", "  permissions: []\n");
    fs::write(defaults.join("hello/docker-compose.yml"), &v1).unwrap();
    m.ensure_defaults(&defaults, Backoff::default())
        .await
        .unwrap();
    m.check_upgrades_now().await;
    assert_eq!(m.installed_plugin(&id(), &zh()).unwrap().upgrade, None);

    let v2 =
        PluginPackage::parse(&hello("1.1.0", "", "  permissions: []\n"), &super::langs()).unwrap();
    let commit = "a".repeat(40);
    let entry = IndexEntry::of(&v2, SOURCE, &commit).unwrap();
    entry.write_to(&index, &v2).unwrap();
    assert_eq!(m.check_upgrades_now().await, None);
    let plugin = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(plugin.upgrade.unwrap().version, "1.1.0");

    upgrade(&m).await.unwrap();
    assert_eq!(image(&docker, &web()), "web:1.1.0");
    let plugin = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(plugin.trust, PluginTrust::Official);
    assert_eq!(plugin.source.as_deref(), Some("github:acme/hello@v1.1.0"));
}

#[tokio::test]
pub(crate) async fn system_plugins_upgrade_from_the_image_and_say_so_when_they_cannot() {
    // 这个 async 块在 debug 构建里很大，2 MiB 的测试线程栈放不下（溢出过一次），放堆上跑。
    Box::pin(async {
        let (docker, engine) = FakeDocker::start().await;
        let dir = tempfile::tempdir().unwrap();
        let m = Arc::new(manager_with(engine, &dir));
        let system = dir.path().join("system");
        fs::create_dir_all(system.join("hello")).unwrap();
        let write = |version: &str, services: &str| {
            let compose = hello(version, services, "  permissions: []\n")
                .replace("kind: app", "kind: system");
            fs::write(system.join("hello/docker-compose.yml"), compose).unwrap();
        };
        write("1.0.0", "");
        m.ensure_system(&system, Backoff::default()).await.unwrap();
        assert_eq!(version(&m), "1.0.0");
        let plugin = m.installed_plugin(&id(), &zh()).unwrap();
        assert!(!plugin.removable, "随核心带的不给接口卸");
        assert_eq!(plugin.trust, PluginTrust::System);
        let err = m
            .preview_upgrade(&id(), UpgradeTo::Offered, None, &zh())
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "unsupported", "接口升不了系统插件：{err}");

        let worker = container_name("app-hello", "worker");
        docker.set_state(
            &worker,
            json!({"Status": "exited", "Running": false, "ExitCode": 1}),
        );
        write("1.1.0", "  worker:\n    image: worker:1\n");
        m.ensure_system(&system, Backoff::default()).await.unwrap();
        let plugin = m.installed_plugin(&id(), &zh()).unwrap();
        assert_eq!(plugin.version, "1.0.0");
        assert!(matches!(plugin.status, PluginState::Running { .. }));
        assert!(
            {
                // 原因里有动态的出错详情，钉住前缀就够：版本、还在跑的版本和“下次启动再试”。
                let prefix = super::message(
                    "plugin-settings-upgrade-bundled-failed",
                    &[("version", "1.1.0"), ("from", "1.0.0"), ("error", "")],
                );
                let prefix = prefix
                    .strip_suffix(": ")
                    .or_else(|| prefix.strip_suffix("："))
                    .unwrap_or(&prefix);
                plugin.notices[0].text.contains(prefix)
            },
            "{:?}",
            plugin.notices
        );

        write("1.2.0", "");
        m.ensure_system(&system, Backoff::default()).await.unwrap();
        let plugin = m.installed_plugin(&id(), &zh()).unwrap();
        assert_eq!(plugin.version, "1.2.0");
        assert!(plugin.notices.is_empty(), "{:?}", plugin.notices);
        assert_eq!(image(&docker, &web()), "web:1.2.0");
    })
    .await;
}

#[tokio::test]
pub(crate) async fn a_system_plugin_installed_from_a_source_can_be_upgraded() {
    let (github, _docker, m, _dir) = with_github(false).await;
    let system = |version: &str| {
        hello(version, "", "  permissions: []\n").replace("kind: app", "kind: system")
    };
    release(&github, &system("1.0.0"));
    install(&m, SOURCE).await;
    assert!(m.is_system(&id()));
    assert!(!m.is_bundled_system(&id()), "从来源装的不算随核心带");

    release(&github, &system("1.1.0"));
    m.check_upgrades_now().await;
    assert!(
        m.installed_plugin(&id(), &zh()).unwrap().upgrade.is_some(),
        "来源装的就该查新版本"
    );
    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Offered, None, &zh())
        .await
        .unwrap();
    assert_eq!(preview.version, "1.1.0");
    upgrade(&m).await.unwrap();
    assert_eq!(version(&m), "1.1.0");
}

#[tokio::test]
pub(crate) async fn a_source_install_with_a_bundled_id_is_not_claimed_by_the_dir() {
    let (github, _docker, m, _dir) = with_github(false).await;
    let source = hello("1.0.0", "", "  permissions: []\n").replace("kind: app", "kind: system");
    release(&github, &source);
    install(&m, SOURCE).await;

    let dir = tempfile::tempdir().unwrap();
    let system = dir.path().join("system");
    fs::create_dir_all(system.join("hello")).unwrap();
    fs::write(
        system.join("hello/docker-compose.yml"),
        hello("1.1.0", "", "  permissions: []\n").replace("kind: app", "kind: system"),
    )
    .unwrap();
    m.ensure_system(&system, Backoff::default()).await.unwrap();

    assert!(!m.is_bundled_system(&id()), "来源装的不该被目录认领");
    assert_eq!(version(&m), "1.0.0", "不从镜像升");
    let plugin = m.installed_plugin(&id(), &zh()).unwrap();
    assert!(plugin.removable);
}

#[tokio::test]
pub(crate) async fn an_uploaded_system_plugin_can_be_upgraded() {
    let (_github, _docker, m, _dir) = with_github(false).await;
    let manifest = PluginPackage::parse(
        &hello("1.0.0", "", "  permissions: []\n").replace("kind: app", "kind: system"),
        &super::langs(),
    )
    .unwrap();
    let permissions = manifest.manifest.permissions.clone();
    m.install_package(manifest, &permissions, super::langs())
        .await
        .unwrap();
    assert!(m.is_system(&id()));
    assert!(!m.is_bundled_system(&id()), "上传的不算随核心带");
    let next = PluginPackage::parse(
        &hello("1.1.0", "", "  permissions: []\n").replace("kind: app", "kind: system"),
        &super::langs(),
    )
    .unwrap();
    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Upload(Box::new(next)), None, &zh())
        .await
        .unwrap();
    assert_eq!(preview.version, "1.1.0");
}

/// 核心升级后读不懂旧记录（清单新增了必填字段，插件版本号却没变）：接口重装不了系统插件，升级要先读旧包也走不通。
/// 启动时保留数据，用随核心带来的包重装。
#[tokio::test]
pub(crate) async fn a_system_plugin_whose_record_the_core_cannot_read_is_reinstalled_from_the_image()
 {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let system = dir.path().join("system");
    fs::create_dir_all(system.join("hello")).unwrap();
    let compose = hello("1.0.0", "", "  permissions: []\n").replace("kind: app", "kind: system");
    fs::write(system.join("hello/docker-compose.yml"), &compose).unwrap();
    let m = manager_with(engine.clone(), &dir);
    m.ensure_system(&system, Backoff::default()).await.unwrap();
    let kept = dir.path().join("apps").join(id().slug()).join("kept");
    fs::create_dir_all(kept.parent().unwrap()).unwrap();
    fs::write(&kept, "主人的数据").unwrap();

    let old = compose.replace("  title: {zh: 你好, en: Hello}\n", "");
    assert!(
        PluginPackage::parse(&old, &super::langs()).is_err(),
        "旧记录这份核心读不懂"
    );
    let mut record = m.store.load(&id()).unwrap().unwrap();
    record.source = old;
    m.store.save(&record).unwrap();

    let m = manager_with(engine, &dir);
    m.restore().await.unwrap();
    assert!(
        matches!(m.list()[0].status, PluginStatus::Degraded { .. }),
        "重启后记录读不懂"
    );
    m.ensure_system(&system, Backoff::default()).await.unwrap();
    assert!(matches!(m.list()[0].status, PluginStatus::Running));
    assert_eq!(version(&m), "1.0.0");
    assert_eq!(m.store.load(&id()).unwrap().unwrap().source, compose);
    assert_eq!(fs::read_to_string(&kept).unwrap(), "主人的数据");
    assert_eq!(image(&docker, &web()), "web:1.0.0");
}
