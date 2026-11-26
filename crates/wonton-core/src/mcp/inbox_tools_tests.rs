//! 收件箱 MCP 工具（ADR-0119 第 3 节）：主人 Bot 能发、管理员发要有 notify、wait 只看自己的条目。

use serde_json::{Value, json};
use wonton_protocol::InboxType;

use super::test_setup::{Setup, setup_notify, setup_optional};
use super::{Caller, PluginId};
use crate::inbox::{Inbox, tools};

fn question() -> Value {
    json!([{
        "id": "q0",
        "header": "目标盘",
        "question": "备份到哪？",
        "options": [{"id": "nas", "label": "NAS"}, {"id": "local", "label": "本地"}]
    }])
}

fn attach(setup: &Setup) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    setup.bus.attach_inbox(Inbox::for_test(dir.path()));
    dir
}

fn text(result: rmcp::model::CallToolResult) -> Value {
    let Some(rmcp::model::ContentBlock::Text(text)) = result.content.first() else {
        panic!("工具结果不是文本")
    };
    serde_json::from_str(&text.text).unwrap()
}

async fn ask(setup: &Setup, audience: &str) -> Result<Value, super::ToolFailure> {
    let result = tools::call(
        &setup.bus,
        &Caller::new(PluginId::parse("app.example.notes").unwrap(), None),
        "inbox_ask",
        json!({"questions": question(), "blocking": false, "audience": audience})
            .as_object()
            .cloned()
            .unwrap(),
    )
    .await?;
    Ok(text(result))
}

/// 主人 Bot（recipient None）也能发，落成主人的收件箱条目，不是 400。
#[tokio::test]
async fn the_owner_bot_can_ask() {
    let setup = setup_notify().await;
    let _dir = attach(&setup);
    let reply = ask(&setup, "keeper").await.unwrap();
    let id = reply["item_id"].as_str().unwrap();
    let inbox = setup.bus.inbox().unwrap();
    let item = inbox
        .view("owner", id, crate::inbox::now_ms())
        .unwrap()
        .item;
    assert_eq!(item.kind, InboxType::Choice);
}

/// 发给所有管理员要 notify；有 notify 通过，没有就 permission_denied。
#[tokio::test]
async fn admins_need_notify_permission() {
    let setup = setup_notify().await;
    let _dir = attach(&setup);
    let reply = ask(&setup, "admins").await.unwrap();
    let id = reply["item_id"].as_str().unwrap();
    let inbox = setup.bus.inbox().unwrap();
    assert_eq!(
        inbox
            .view("owner", id, crate::inbox::now_ms())
            .unwrap()
            .item
            .status,
        wonton_protocol::InboxStatus::Pending
    );

    let plain = setup_optional().await;
    let _dir = attach(&plain);
    let error = ask(&plain, "admins").await.unwrap_err();
    assert_eq!(error.kind, "permission_denied");
}

/// 别的 Bot 知道 id 也 wait 不到。
#[tokio::test]
async fn wait_checks_the_sender() {
    let setup = setup_notify().await;
    let _dir = attach(&setup);
    let reply = ask(&setup, "keeper").await.unwrap();
    let id = reply["item_id"].as_str().unwrap().to_owned();
    let other = Caller::new(PluginId::parse("com.example.other").unwrap(), None);
    let error = tools::call(
        &setup.bus,
        &other,
        "inbox_wait",
        json!({"item_id": id}).as_object().cloned().unwrap(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, "not_yours");
}
