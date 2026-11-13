//! `wonton__user_locale`（ADR-0043）：按人取语言，默认语言由插件自己定；查别人要有读家人名单的权限
//! （`owner.identity`）。

use std::time::Duration;

use serde_json::json;
use wonton_protocol::MemberStatus;

use super::Caller;
use super::test_setup::{Setup, setup};
use super::tests::{failed, text};
use crate::members::MemberRecord;
use crate::plugin::docker::Engine;
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, PluginPackage, PluginStatus};

const TOOL: &str = "wonton__user_locale";
const ALICE: &str = "@alice:wonton.localhost";

/// 有 `owner.identity` 的插件：读家人名单用的就是这条权限。
const IDENTITY: &str = "name: identity
services:
  mcp:
    image: example/identity
x-wonton:
  spec: '0.5'
  id: app.example.identity
  version: 0.1.0
  kind: app
  title: {zh: 身份, en: Identity}
  description: {zh: 身份, en: Identity}
  permissions: [owner.identity]
";

fn invite(s: &Setup, username: &str) {
    s.members
        .invite(MemberRecord {
            username: username.into(),
            user_id: format!("@{username}:wonton.localhost"),
            display_name: username.into(),
            email: None,
            status: MemberStatus::Joined,
            role_id: "friend".to_owned(),
            wants_private_bot: false,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        })
        .unwrap();
}

fn identity_caller(s: &Setup) -> Caller {
    let manifest =
        PluginPackage::manifest_of(IDENTITY, &Languages::from_header(Some("en"))).unwrap();
    s.manager
        .register_for_test(manifest, PluginStatus::Running, None);
    Caller::new(PluginId::parse("app.example.identity").unwrap(), None)
}

#[tokio::test]
async fn the_owner_is_the_default() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    let result = s.call(TOOL, json!({})).await;
    assert!(!failed(&result), "{}", text(&result));
    let value: serde_json::Value = serde_json::from_str(&text(&result)).unwrap();
    assert_eq!(value["user_id"], "@owner:wonton.localhost");
    // 夹具记的主人是 zh。
    assert_eq!(value["locale"], "zh");

    s.manager
        .note_person_languages("owner", Some("zh-Hant-TW, en;q=0.8, *;q=0.5"));
    let result = s.call(TOOL, json!({})).await;
    let value: serde_json::Value = serde_json::from_str(&text(&result)).unwrap();
    assert_eq!(value["locale"], "zh-Hant-TW", "原样，不收窄");
    assert_eq!(value["languages"], json!(["zh-Hant-TW", "en"]));

    let tools = s.bus.tools(&s.caller).await;
    let tool = tools.iter().find(|t| t.name == TOOL).unwrap();
    assert_eq!(
        tool.annotations.as_ref().and_then(|a| a.read_only_hint),
        Some(true),
        "只读"
    );
}

#[tokio::test]
async fn other_people_need_the_family_list_permission() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    invite(&s, "alice");
    s.manager
        .note_person_languages("alice", Some("ja-JP, en;q=0.5"));
    s.manager.note_person_languages("owner", Some("zh-Hans"));

    // 没注册、没权限的调用方：查家人回权限不够，查主人照旧可以。
    let denied = s.call(TOOL, json!({"user_id": ALICE})).await;
    assert!(failed(&denied));
    assert!(
        text(&denied).starts_with("permission_denied"),
        "{}",
        text(&denied)
    );
    let owner = s
        .call(TOOL, json!({"user_id": "@owner:wonton.localhost"}))
        .await;
    assert!(!failed(&owner), "{}", text(&owner));

    let caller = identity_caller(&s);
    let result = s.call_as(&caller, TOOL, json!({"user_id": ALICE})).await;
    assert!(!failed(&result), "{}", text(&result));
    let value: serde_json::Value = serde_json::from_str(&text(&result)).unwrap();
    assert_eq!(value["user_id"], ALICE);
    assert_eq!(value["locale"], "ja-JP");
    assert_eq!(value["languages"], json!(["ja-JP", "en"]));
}

/// 没有权限时，存在的人和不存在的人回同一个错：不申请权限就探不出家里有谁。
#[tokio::test]
async fn without_permission_everyone_else_looks_the_same() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    invite(&s, "alice");
    for id in [ALICE, "@nobody:wonton.localhost"] {
        let result = s.call(TOOL, json!({"user_id": id})).await;
        assert!(failed(&result));
        assert!(
            text(&result).starts_with("permission_denied"),
            "{id}: {}",
            text(&result)
        );
    }
}

/// 没有记录的人回 null 和空数组，不是主人的语言。
#[tokio::test]
async fn a_person_without_a_record_gets_null() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    invite(&s, "bob");
    s.manager.note_person_languages("owner", Some("zh-Hans"));
    s.manager.note_person_languages("bob", None);
    let caller = identity_caller(&s);
    let result = s
        .call_as(&caller, TOOL, json!({"user_id": "@bob:wonton.localhost"}))
        .await;
    assert!(!failed(&result), "{}", text(&result));
    let value: serde_json::Value = serde_json::from_str(&text(&result)).unwrap();
    assert_eq!(value["locale"], serde_json::Value::Null);
    assert_eq!(value["languages"], json!([]));
}

#[tokio::test]
async fn other_servers_and_strangers_have_their_own_errors() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    invite(&s, "gone");
    s.members.remove("gone", false, false).unwrap();
    let caller = identity_caller(&s);
    let call = |id: &'static str| s.call_as(&caller, TOOL, json!({"user_id": id}));

    let elsewhere = call("@alice:elsewhere.org").await;
    assert!(failed(&elsewhere));
    assert!(
        text(&elsewhere).starts_with("not_this_server"),
        "{}",
        text(&elsewhere)
    );

    // 本服务器上的陌生人（不是家人记录里的人）。
    let stranger = call("@nobody:wonton.localhost").await;
    assert!(failed(&stranger));
    assert!(
        text(&stranger).starts_with("not_found"),
        "{}",
        text(&stranger)
    );

    // 已移除的家人也找不到。
    let removed = call("@gone:wonton.localhost").await;
    assert!(failed(&removed));
    assert!(
        text(&removed).starts_with("not_found"),
        "{}",
        text(&removed)
    );
}
