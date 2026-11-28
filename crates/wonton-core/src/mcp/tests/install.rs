//! Bot 安装插件：只从官方索引装，要主人确认；索引读不了如实报错。

use super::*;

#[tokio::test]
async fn the_bot_installs_from_the_official_index_after_the_owner_confirms() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    let found = s
        .call("wonton__search_catalog", json!({"query": "日记"}))
        .await;
    let found: Value = serde_json::from_str(&text(&found)).unwrap();
    assert_eq!(found["plugins"][0]["id"], "app.example.journal");
    // 主人没存过语言，回退英文；期望值用目录本身格式化出来。
    let langs = s.bus.plugins.owner_languages();
    let share = crate::l10n::text(
        &langs,
        "permission-storage-share",
        &[("access", "read".into()), ("name", "documents".into())],
    );
    assert_eq!(found["plugins"][0]["permissions"], json!([share]));

    let call = s.spawn(
        "wonton__install_plugin",
        json!({"id": "app.example.journal"}),
    );
    let card = s.cards.wait_for(1).await;
    assert!(
        card.title.contains("测试 Bot") && card.title.contains("日记"),
        "{}",
        card.title
    );
    let lines: Vec<&str> = card.lines.iter().map(|l| l.text.as_str()).collect();
    assert!(lines.contains(&format!("· {share}").as_str()), "{lines:?}");
    assert!(
        s.manager
            .list()
            .iter()
            .all(|p| p.id.as_str() != "app.example.journal")
    );
    s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    let installed = s.manager.list();
    let journal = installed
        .iter()
        .find(|p| p.id.as_str() == "app.example.journal");
    assert_eq!(
        journal.map(|p| p.status.clone()),
        Some(PluginStatus::Running)
    );
    assert!(
        matches!(
            s.bus.confirmations().card(&card.id).map(|card| card.state),
            Some(ConfirmationState::Succeeded { .. })
        ),
        "装完的卡片是结果"
    );

    // 已经装了、官方索引里没有、要核心不开放的权限的：直接拒绝，不发卡片。
    for (id, reason) in [
        ("app.example.journal", "已经装了"),
        ("app.example.nope", "官方索引里没有"),
        ("app.example.root", "docker.sock 挂载"),
    ] {
        let result = s.call("wonton__install_plugin", json!({"id": id})).await;
        assert!(
            failed(&result) && text(&result).contains(reason),
            "{id}: {result:?}"
        );
    }
    assert_eq!(s.cards.posted.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_bot_installs_a_high_permission_plugin_once_the_owner_approves_the_card() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;

    let call = s.spawn("wonton__install_plugin", json!({"id": "app.example.nas"}));
    let card = s.cards.wait_for(1).await;
    assert!(
        card.lines.iter().any(|l| l.emphasis),
        "高权限要醒目：{:?}",
        card.lines
    );
    s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert!(
        s.manager
            .list()
            .iter()
            .any(|p| p.id.as_str() == "app.example.nas")
    );
}

#[tokio::test]
async fn an_unreadable_index_is_reported_not_shown_as_empty() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    fs::write(s.dir.path().join("index").join("index.json"), "{").unwrap();
    for (tool, args) in [
        ("wonton__search_catalog", json!({})),
        (
            "wonton__install_plugin",
            json!({"id": "app.example.journal"}),
        ),
    ] {
        let result = s.call(tool, args).await;
        assert!(
            failed(&result) && text(&result).contains("官方索引用不了"),
            "{tool}: {result:?}"
        );
    }
    assert!(s.cards.posted.lock().unwrap().is_empty());
}
