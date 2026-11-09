//! 工具失败和确认的契约（#437）：给 Agent 的失败结果一律 `isError: true`，`_meta` 的 `app.wonton/failure`
//! 带类别；主人确认过才执行的成功结果带 `app.wonton/confirmation`。断言字段和类别，不断言整段文案。

use std::time::Duration;

use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use wonton_protocol::{Answer, ConfirmationState};

use super::test_setup::*;
use super::tests::{failed, text};
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::languages::Languages;
use crate::plugin::package_files::PackageFiles;
use crate::plugin::storefront::index::IndexEntry;
use crate::plugin::{PluginId, PluginPackage, PluginStatus};

pub(super) fn failure_kind(result: &CallToolResult) -> Option<&str> {
    result
        .meta
        .as_ref()
        .and_then(|m| m.0.get("app.wonton/failure"))
        .and_then(Value::as_str)
}

pub(super) fn confirmation(result: &CallToolResult) -> Option<&str> {
    result
        .meta
        .as_ref()
        .and_then(|m| m.0.get("app.wonton/confirmation"))
        .and_then(Value::as_str)
}

/// 索引里的日记插件注册成已安装：测“已经装了”这条拒绝。
fn register_journal_installed(s: &Setup) {
    let source = official_package(
        "app.example.journal",
        "日记",
        "storage.share.documents:read",
    );
    let package = PluginPackage::parse(&source, &Languages::from_header(Some("zh-Hans"))).unwrap();
    s.manager
        .register_for_test(package.manifest, PluginStatus::Running, None);
}

#[tokio::test]
async fn install_denials_carry_is_error_and_a_category() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;

    // 官方索引里没有：not_found。
    let result = s
        .call("wonton__install_plugin", json!({"id": "app.example.nope"}))
        .await;
    assert!(failed(&result), "{result:?}");
    assert_eq!(failure_kind(&result), Some("not_found"));

    // 已经装了：conflict。
    register_journal_installed(&s);
    let result = s
        .call(
            "wonton__install_plugin",
            json!({"id": "app.example.journal"}),
        )
        .await;
    assert!(failed(&result), "{result:?}");
    assert_eq!(failure_kind(&result), Some("conflict"));

    // id 和 dir 一起给：invalid_input。
    let result = s
        .call(
            "wonton__install_plugin",
            json!({"id": "app.example.journal", "dir": "notes"}),
        )
        .await;
    assert!(failed(&result), "{result:?}");
    assert_eq!(failure_kind(&result), Some("invalid_input"));
}

#[tokio::test]
async fn failing_catalog_and_package_fetches_carry_categories() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;

    // 整份索引读不了：source_unavailable，不是“索引是空的”。
    let index = s.dir.path().join("index").join("index.json");
    let away = s.dir.path().join("index").join("index.json.away");
    std::fs::rename(&index, &away).unwrap();
    let result = s
        .call(
            "wonton__install_plugin",
            json!({"id": "app.example.journal"}),
        )
        .await;
    std::fs::rename(&away, &index).unwrap();
    assert!(failed(&result), "{result:?}");
    assert_eq!(failure_kind(&result), Some("source_unavailable"));
    assert_eq!(s.cards.posted.lock().unwrap().len(), 0, "失败时不发卡片");

    // 索引读得到、插件包的文件取不到：装包这步失败，同样带类别、不发卡片。
    let source = format!(
        "{}  icon: app.png\n",
        official_package(
            "app.example.fetchfail",
            "取包失败",
            "storage.share.documents:read"
        )
    );
    let files = PackageFiles::from([("app.png".to_owned(), [0x89, b'P', b'N', b'G'].to_vec())]);
    let package =
        PluginPackage::with_files(&source, files, &Languages::from_header(Some("zh-Hans")))
            .unwrap();
    let entry = IndexEntry::of(&package, "github:example/fetchfail", &"a".repeat(40)).unwrap();
    entry
        .write_to(&s.dir.path().join("index"), &package)
        .unwrap();
    let blob = s
        .dir
        .path()
        .join("index")
        .join("blobs")
        .join(entry.files.get("app.png").unwrap());
    std::fs::remove_file(&blob).unwrap();
    let result = s
        .call(
            "wonton__install_plugin",
            json!({"id": "app.example.fetchfail"}),
        )
        .await;
    assert!(failed(&result), "{result:?}");
    assert_eq!(failure_kind(&result), Some("not_found"));
    assert_eq!(s.cards.posted.lock().unwrap().len(), 0, "失败时不发卡片");
}

#[tokio::test]
async fn rejected_expired_and_unavailable_confirmations_carry_kinds() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_millis(400)).await;

    // 主人点了拒绝。
    let call = s.spawn(
        "wonton__install_plugin",
        json!({"id": "app.example.journal"}),
    );
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Reject).unwrap();
    let result = call.await.unwrap();
    assert!(failed(&result), "{result:?}");
    assert_eq!(failure_kind(&result), Some("owner_rejected"));
    assert_eq!(
        s.cards.states(&card.id).last(),
        Some(&ConfirmationState::Rejected)
    );

    // 主人没在期限内回答：过期。
    let call = s.spawn(
        "wonton__install_plugin",
        json!({"id": "app.example.journal"}),
    );
    s.cards.wait_for(2).await;
    let result = call.await.unwrap();
    assert!(failed(&result), "{result:?}");
    assert_eq!(failure_kind(&result), Some("confirmation_expired"));

    // 卡片发不出去：没能请主人确认。
    s.cards
        .broken
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let result = s
        .call(
            "wonton__install_plugin",
            json!({"id": "app.example.journal"}),
        )
        .await;
    s.cards
        .broken
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(failed(&result), "{result:?}");
    assert_eq!(failure_kind(&result), Some("confirmation_unavailable"));
}

/// 等主人确认中没有工具结果：调用一直挂着，卡片先到。
#[tokio::test]
async fn while_awaiting_the_owner_there_is_no_tool_result() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    let call = s.spawn(
        "wonton__install_plugin",
        json!({"id": "app.example.journal"}),
    );
    let card = s.cards.wait_for(1).await;
    assert!(!call.is_finished(), "卡片还在等回答，调用不该已经有结果");
    s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{}", text(&result));
}

#[tokio::test]
async fn an_approved_install_says_it_went_through_the_owner() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    let call = s.spawn(
        "wonton__install_plugin",
        json!({"id": "app.example.journal"}),
    );
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{}", text(&result));
    assert_eq!(confirmation(&result), Some("card"), "{result:?}");
}

#[tokio::test]
async fn a_call_riding_a_window_says_so() {
    let s = setup_window(Duration::from_secs(60)).await;
    // 主人点了“允许并放行时段”：这次走的是它自己的卡片，confirmation 是 card。
    let first = s.spawn("app-example-notes__delete", json!({"name": "a"}));
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::AllowWindow).unwrap();
    let result = first.await.unwrap();
    assert!(!failed(&result), "{}", text(&result));
    assert_eq!(confirmation(&result), Some("card"), "{result:?}");

    // 时段里的第二次调用不发卡片，靠的是主人先前批准的时段：confirmation 是 window。
    let result = s
        .call("app-example-notes__delete", json!({"name": "b"}))
        .await;
    assert!(!failed(&result), "{}", text(&result));
    assert_eq!(confirmation(&result), Some("window"), "{result:?}");
}

/// 插件没起来（连接不上）：不是“插件说没有这个工具”，结果说清是够不着，晚点再试。
#[tokio::test]
async fn a_plugin_tool_of_a_dead_plugin_reports_source_unavailable() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    let notes = PluginId::parse("app.example.notes").unwrap();
    s.manager
        .mcp_for_test(&notes, "http://127.0.0.1:1/mcp".to_owned());
    let result = s
        .call("app-example-notes__search", json!({"name": "x"}))
        .await;
    assert!(failed(&result), "{result:?}");
    assert_eq!(failure_kind(&result), Some("source_unavailable"));
}

/// 插件的工具执行时自己报错：类别是插件出错（上游错误），和够不着插件分得开，别盲目重试。
#[tokio::test]
async fn a_plugin_tool_erroring_reports_upstream_error() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    let result = s.call("app-example-notes__boom", json!({})).await;
    assert!(failed(&result), "{result:?}");
    assert_eq!(failure_kind(&result), Some("upstream_error"));
}

/// 主人点了允许、装这步失败：结果如实是失败，类别说清装到哪一步出的事；确认环节也如实标着。
#[tokio::test]
async fn an_approved_install_that_fails_reports_the_failure_kind() {
    let (docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    // 别的插件先占了 journal 的网络名：卡片前的检查不碰 Docker 的网络，批准后建网络这步才失败，
    // 类别是 conflict（名字被别的应用占了）。
    docker.add_network("wonton-app-example-journal", "app.example.other", json!({}));
    let call = s.spawn(
        "wonton__install_plugin",
        json!({"id": "app.example.journal"}),
    );
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(failed(&result), "{}", text(&result));
    assert_eq!(confirmation(&result), Some("card"), "{result:?}");
    assert_eq!(failure_kind(&result), Some("conflict"));
}
