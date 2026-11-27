use super::*;

#[tokio::test]
pub(crate) async fn a_new_tag_is_offered_and_the_upgrade_keeps_the_data() {
    let (github, docker, m, _dir) = with_github(false).await;
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    m.check_upgrades_now().await;
    assert_eq!(m.installed_plugin(&id(), &zh()).unwrap().upgrade, None);

    release(
        &github,
        &hello(
            "1.1.0",
            "  worker:\n    image: worker:1\n",
            "  permissions: []\n",
        ),
    );
    release(&github, &hello("2.0.0-rc.1", "", "  permissions: []\n"));
    assert_eq!(m.check_upgrades_now().await, None);
    let offer = m.installed_plugin(&id(), &zh()).unwrap().upgrade.unwrap();
    assert_eq!(offer.version, "1.1.0", "预发布版本不提示");
    assert_eq!(offer.release_notes.as_deref(), Some("版本 1.1.0 的说明"));
    assert!(!offer.permissions_changed);

    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Offered, None, &zh())
        .await
        .unwrap();
    assert_eq!(preview.upgrade_from.as_deref(), Some("1.0.0"));
    assert_eq!(preview.version, "1.1.0");

    let before = docker.writes().len();
    upgrade(&m).await.unwrap();
    let plugin = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(plugin.version, "1.1.0");
    assert_eq!(plugin.upgrade, None);
    assert!(matches!(plugin.status, PluginState::Running { .. }));
    assert_eq!(plugin.source.as_deref(), Some("github:acme/hello@v1.1.0"));
    assert_eq!(image(&docker, &web()), "web:1.1.0");
    assert!(
        docker
            .containers()
            .contains(&container_name("app-hello", "worker"))
    );
    let writes = docker.writes();
    assert!(
        !writes[before..]
            .iter()
            .any(|w| w.starts_with("DELETE /volumes")),
        "数据卷不动：{writes:?}"
    );
    let record = m.store.load(&id()).unwrap().unwrap();
    assert_eq!(record.version, "1.1.0");
    assert_eq!(record.origin.unwrap().tag.as_deref(), Some("v1.1.0"));
    assert_eq!(
        m.preview_upgrade(&id(), UpgradeTo::Offered, None, &zh())
            .await
            .unwrap_err()
            .kind(),
        "not_found",
        "已经是最新的"
    );
}

#[tokio::test]
pub(crate) async fn stopped_plugins_are_offered_new_versions() {
    let (github, docker, m, _dir) = with_github(false).await;
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    let revision = m.installed_plugin(&id(), &zh()).unwrap().revision;
    m.stop_detached(id()).await.unwrap();
    assert_eq!(
        m.installed_plugin(&id(), &zh()).unwrap().status,
        PluginState::Stopped
    );

    release(&github, &hello("1.1.0", "", "  permissions: []\n"));
    assert_eq!(m.check_upgrades_now().await, None);
    let plugin = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(plugin.status, PluginState::Stopped);
    assert_eq!(plugin.upgrade.unwrap().version, "1.1.0");
    assert_eq!(plugin.revision, revision);
    let before = docker.writes();
    assert!(matches!(upgrade(&m).await, Err(InstallError::Conflict(_))));
    assert_eq!(docker.writes(), before);
    assert_eq!(version(&m), "1.0.0");
    assert_eq!(
        m.lifecycle_status(&id()).unwrap().desired,
        wonton_protocol::PluginDesiredState::Stopped
    );
}

#[tokio::test]
pub(crate) async fn a_rewritten_tag_is_reported_and_not_an_upgrade() {
    let (github, _docker, m, _dir) = with_github(false).await;
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    release(
        &github,
        &hello("1.0.0", "", "  permissions: []\n  license: MIT\n"),
    );
    m.check_upgrades_now().await;
    let plugin = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(plugin.upgrade, None);
    assert!(
        plugin.notices[0].text.contains("v1.0.0"),
        "{:?}",
        plugin.notices
    );
    let err = m
        .preview_upgrade(&id(), UpgradeTo::Offered, None, &zh())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "not_found", "{err}");
}

#[tokio::test]
pub(crate) async fn plugins_without_a_tag_are_not_checked_and_uploads_are_not_offered_upgrades() {
    let (github, docker, m, _dir) = with_github(false).await;
    let commit = release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, &format!("{SOURCE}@{commit}")).await;
    release(&github, &hello("1.1.0", "", "  permissions: []\n"));
    m.check_upgrades_now().await;
    assert_eq!(m.installed_plugin(&id(), &zh()).unwrap().upgrade, None);

    let parse = |compose: &str| PluginPackage::parse(compose, &super::langs()).unwrap();
    let newer = hello("1.2.0", "", "  permissions: []\n");
    m.upgrade_upload(
        &id(),
        parse(&newer),
        Vec::new(),
        Vec::new(),
        false,
        &[],
        &Default::default(),
        &super::langs(),
    )
    .await
    .unwrap();
    assert_eq!(image(&docker, &web()), "web:1.2.0");
    assert_eq!(
        m.installed_plugin(&id(), &zh()).unwrap().trust,
        PluginTrust::Local,
        "上传的是本地来源，以后不查新版本"
    );

    let other = hello("1.3.0", "", "  permissions: []\n").replace("id: app.hello", "id: app.other");
    let err = m
        .upgrade_upload(
            &id(),
            parse(&other),
            Vec::new(),
            Vec::new(),
            false,
            &[],
            &Default::default(),
            &super::langs(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "invalid_input", "{err}");
}
