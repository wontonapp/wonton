//! Bot 卸载插件：要主人确认，不能卸自己；系统插件装了也能卸。

use super::*;

#[tokio::test]
async fn uninstalling_needs_the_owner_and_never_removes_the_caller_itself() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    let package = PluginPackage::parse(
        &official_package(
            "app.example.journal",
            "日记",
            "storage.share.documents:read",
        ),
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    let permissions = package.manifest.permissions.clone();
    s.manager
        .install_package(
            package,
            &permissions,
            Languages::from_header(Some("zh-Hans")),
        )
        .await
        .unwrap();

    let call = s.spawn(
        "wonton__uninstall_plugin",
        json!({"id": "app.example.journal", "delete_data": true}),
    );
    let card = s.cards.wait_for(1).await;
    assert!(
        card.lines
            .iter()
            .any(|l| l.emphasis && l.text.contains("删除它的数据"))
    );
    s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert!(
        s.manager
            .list()
            .iter()
            .all(|p| p.id.as_str() != "app.example.journal")
    );

    let manifest = PluginPackage::parse(
        &NOTES.replace("app.example.notes", "com.example.agent"),
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap()
    .manifest;
    s.manager
        .register_for_test(manifest, PluginStatus::Running, None);
    let result = s
        .call(
            "wonton__uninstall_plugin",
            json!({"id": "com.example.agent"}),
        )
        .await;
    assert!(
        failed(&result) && text(&result).contains("不能卸载自己"),
        "{result:?}"
    );
    assert_eq!(s.cards.posted.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_source_installed_system_plugin_can_be_uninstalled_by_a_bot() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    let id = "app.example.system";
    let package = PluginPackage::parse(
        &official_package(id, "系统应用", "").replace("kind: app", "kind: system"),
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    let permissions = package.manifest.permissions.clone();
    s.manager
        .install_package(
            package,
            &permissions,
            Languages::from_header(Some("zh-Hans")),
        )
        .await
        .unwrap();
    let mut record = s
        .manager
        .store()
        .load(&PluginId::parse(id).unwrap())
        .unwrap()
        .unwrap();
    record.origin = Some(crate::plugin::github_source::Origin {
        repo: "github:acme/system".into(),
        tag: Some("v1.0.0".into()),
        commit: "0".repeat(40),
        trust: Default::default(),
        index: None,
    });
    s.manager.store().save(&record).unwrap();

    let call = s.spawn("wonton__uninstall_plugin", json!({"id": id}));
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
}

#[tokio::test]
async fn an_uploaded_system_plugin_can_be_uninstalled_by_a_bot() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    let id = "app.example.uploaded";
    let package = PluginPackage::parse(
        &official_package(id, "上传的系统应用", "").replace("kind: app", "kind: system"),
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    let permissions = package.manifest.permissions.clone();
    s.manager
        .install_package(
            package,
            &permissions,
            Languages::from_header(Some("zh-Hans")),
        )
        .await
        .unwrap();

    let call = s.spawn("wonton__uninstall_plugin", json!({"id": id}));
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Approve).unwrap();
    assert!(!failed(&call.await.unwrap()));
}
