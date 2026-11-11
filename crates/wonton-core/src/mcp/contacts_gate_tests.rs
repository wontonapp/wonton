//! 总线按联系人挡插件工具的行为测试（ADR-0050「远端联系人默认不能用插件」）：有 Active 远端联系人的
//! Bot 列不到插件工具、调用被拒（类别 `remote_contact`），核心自己的工具照给；没有远端联系人的
//! Bot 照常拿到插件工具。

use std::sync::Arc;
use std::time::Duration;

use rmcp::model::CallToolResult;
use serde_json::json;

use super::result::FAILURE_META;
use super::server::BotIdentity;
use super::test_setup::{Setup, setup};
use crate::contacts::Contacts;
use crate::contacts::store::StoredContact;
use crate::contacts::test_support::{FakeBots, FakePeers, FakePush};
use crate::plugin::docker::Engine as Docker;
use wonton_protocol::{BotKeeper, BotRef, ContactKind, ContactState};

/// 给 caller 的 Bot 塞一行 Active 的远端联系人（不走握手；状态机的行为在 `contacts` 的测试里），挂上总线。
async fn setup_with_active_remote() -> (Setup, BotRef) {
    let s = setup(Docker::unreachable_for_test(), Duration::from_secs(60)).await;
    // 门按 caller 认出的 Bot 查（BotIdentity.bot），种子行必须进同一个键下。
    let bot = BotRef {
        plugin_id: s.caller.id.to_string(),
        bot: "helper".to_owned(),
    };
    let peers = Arc::new(FakePeers::default());
    let bots = Arc::new(FakeBots::default());
    bots.add("@helper:wonton.localhost", &bot, "阿福", "云吞", "owner");
    let contacts = Contacts::open(s.dir.path().join("contacts.json"), peers)
        .unwrap()
        .with_bots(bots)
        .with_push(Arc::new(FakePush::default()));
    contacts.seed_row_for_test(
        &bot,
        StoredContact {
            user_id: "@helper:other.example".to_owned(),
            kind: ContactKind::Bot,
            state: ContactState::Active,
            remote: Some("other.example".to_owned()),
            display_name: None,
            keeper_display_name: None,
            request_id: None,
            room: None,
            updated_at_ms: 1,
        },
    );
    s.bus.attach_contacts(contacts);
    (s, bot)
}

/// caller 的 Bot 身份（总线按它认联系人）。
fn caller_with_bot(s: &Setup) -> super::Caller {
    let mut caller = s.caller.clone();
    caller.bot = Some(BotIdentity {
        bot: "helper".into(),
        keeper: BotKeeper::User {
            username: "owner".into(),
        },
    });
    caller
}

#[tokio::test]
async fn remote_contact_bots_lose_plugin_tools() {
    let (s, _bot) = setup_with_active_remote().await;
    let caller = caller_with_bot(&s);
    // 列工具：插件（app.example.notes）的工具一个都不在，核心的只读工具照给，副作用的不给。
    let tools = s.bus.tools(&caller).await;
    assert!(
        tools
            .iter()
            .all(|tool| !tool.name.starts_with("app-example-notes")),
        "插件工具不该出现在名单里：{:?}",
        tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
    );
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert!(
        names.contains(&"wonton__list_plugins") && names.contains(&"wonton__help"),
        "核心的只读工具照给：{:?}",
        names
    );
    assert!(
        !names.contains(&"wonton__install_plugin")
            && !names.contains(&"wonton__save_file")
            && !names.contains(&"wonton__set_profile"),
        "核心的副作用工具不给：{:?}",
        names
    );
    // 直接调也兜住：插件的失败类别写进结果的 meta（机器可读，不是给模型的文案）。
    let result: CallToolResult = s
        .call_as(&caller, "app-example-notes__find", json!({}))
        .await;
    let kind = result
        .meta
        .as_ref()
        .and_then(|meta| meta.0.iter().find(|(key, _)| key.as_str() == FAILURE_META))
        .map(|(_, value)| value.clone());
    assert_eq!(kind, Some(json!("remote_contact")));
    // 核心的副作用工具直接调也挡住；只读的照常通。
    let result: CallToolResult = s.call_as(&caller, "wonton__save_file", json!({})).await;
    let kind = result
        .meta
        .as_ref()
        .and_then(|meta| meta.0.iter().find(|(key, _)| key.as_str() == FAILURE_META))
        .map(|(_, value)| value.clone());
    assert_eq!(kind, Some(json!("remote_contact")));
    let result: CallToolResult = s.call_as(&caller, "wonton__list_plugins", json!({})).await;
    assert!(!result.is_error.unwrap_or(false), "只读的核心工具照常可用");
}

#[tokio::test]
async fn bots_without_remote_contacts_keep_plugin_tools() {
    let s = setup(Docker::unreachable_for_test(), Duration::from_secs(60)).await;
    let peers = Arc::new(FakePeers::default());
    let contacts = Contacts::open(s.dir.path().join("contacts.json"), peers)
        .unwrap()
        .with_bots(Arc::new(FakeBots::default()))
        .with_push(Arc::new(FakePush::default()));
    s.bus.attach_contacts(contacts);
    let caller = caller_with_bot(&s);
    let tools = s.bus.tools(&caller).await;
    assert!(
        tools
            .iter()
            .any(|tool| tool.name.starts_with("app-example-notes")),
        "没有远端联系人时插件工具应该在：{:?}",
        tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
    );
}
