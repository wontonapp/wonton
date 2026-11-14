//! `tools/call` 转给插件时的调用者身份（ADR-0088 第 8 节）：私人 Bot 的 `_meta` 同时带
//! `app.wonton/user` 和 `app.wonton/capabilities`，主人和这个应用里的管理员带 `plugin_admin`；
//! 共享 Bot 和认不出的人两个都不带。

use std::time::Duration;

use serde_json::json;
use wonton_protocol::{
    AdminScope, AppScope, BotKeeper, Permissions, RoleDraft, TOOL_CALL_CAPABILITIES_META,
    TOOL_CALL_USER_META,
};

use super::Caller;
use super::test_setup::{Setup, setup};
use super::tool_failure_tests::failure_kind;
use crate::members::MemberRecord;
use crate::plugin::PluginId;
use crate::plugin::fake_docker::FakeDocker;

const AGENT: &str = "com.example.agent";
const NOTES: &str = "app.example.notes";

fn private_bot(username: &str) -> Caller {
    Caller::for_bot(
        PluginId::parse(AGENT).unwrap(),
        None,
        "main".into(),
        BotKeeper::User {
            username: username.to_owned(),
        },
    )
}

fn shared_bot() -> Caller {
    Caller::for_bot(
        PluginId::parse(AGENT).unwrap(),
        None,
        "main".into(),
        BotKeeper::Household,
    )
}

/// 记一个加入了的人，角色用记录里的 ID。
fn joined(s: &Setup, username: &str, role_id: &str) {
    s.members
        .invite(MemberRecord {
            role_id: role_id.to_owned(),
            ..MemberRecord::for_test(username)
        })
        .unwrap();
}

async fn notes_setup() -> Setup {
    let (_docker, engine) = FakeDocker::start().await;
    setup(engine, Duration::from_secs(5)).await
}

/// 以这个 Bot 的身份调一次只读工具，回插件收到的最后一份 `_meta`（没收到调用或没带就是 `None`）。
async fn call_meta(s: &Setup, caller: &Caller) -> Option<rmcp::model::MetaObject> {
    s.call_as(caller, "app-example-notes__search", json!({}))
        .await;
    s.notes
        .received_meta
        .lock()
        .unwrap()
        .last()
        .and_then(Clone::clone)
}

#[tokio::test]
async fn private_bots_call_as_their_keeper_with_capabilities() {
    let s = notes_setup().await;

    // 主人的私人 Bot：用户名加 `plugin_admin`。
    let meta = call_meta(&s, &private_bot("owner"))
        .await
        .expect("私人 Bot 的调用带身份元数据");
    assert_eq!(meta.0[TOOL_CALL_USER_META], "owner");
    assert_eq!(meta.0[TOOL_CALL_CAPABILITIES_META], json!(["plugin_admin"]));

    // “管理员”模板的成员：同样带能力。
    joined(&s, "ada", "admin");
    let meta = call_meta(&s, &private_bot("ada"))
        .await
        .expect("私人 Bot 的调用带身份元数据");
    assert_eq!(meta.0[TOOL_CALL_USER_META], "ada");
    assert_eq!(meta.0[TOOL_CALL_CAPABILITIES_META], json!(["plugin_admin"]));

    // 能打开应用但不是管理员的成员：带用户名，能力列表是空的。
    let role = s
        .bus
        .people
        .get()
        .unwrap()
        .roles
        .create(&RoleDraft {
            name: "只打开不当管理员".into(),
            permissions: Permissions {
                open_apps: AppScope::Selected {
                    plugin_ids: vec![NOTES.into()],
                },
                ..Permissions::strictest()
            },
        })
        .unwrap();
    joined(&s, "ben", &role.id);
    let meta = call_meta(&s, &private_bot("ben"))
        .await
        .expect("私人 Bot 的调用带身份元数据");
    assert_eq!(meta.0[TOOL_CALL_USER_META], "ben");
    assert_eq!(meta.0[TOOL_CALL_CAPABILITIES_META], json!([]));
}

#[tokio::test]
async fn plugin_admin_capability_follows_the_admin_scope() {
    let s = notes_setup().await;
    let mut draft = RoleDraft {
        name: "只当这个应用的管理员".into(),
        permissions: Permissions {
            open_apps: AppScope::Selected {
                plugin_ids: vec![NOTES.into()],
            },
            plugin_admin: AdminScope::Selected {
                plugin_ids: vec![NOTES.into()],
            },
            ..Permissions::strictest()
        },
    };
    let role = s.bus.people.get().unwrap().roles.create(&draft).unwrap();
    joined(&s, "caro", &role.id);
    let meta = call_meta(&s, &private_bot("caro"))
        .await
        .expect("私人 Bot 的调用带身份元数据");
    assert_eq!(meta.0[TOOL_CALL_USER_META], "caro");
    assert_eq!(meta.0[TOOL_CALL_CAPABILITIES_META], json!(["plugin_admin"]));

    // 管理员范围不再圈这个应用（应用照旧能打开）：能力没了，只剩用户名。
    draft.permissions.plugin_admin = AdminScope::Selected {
        plugin_ids: Vec::new(),
    };
    s.bus
        .people
        .get()
        .unwrap()
        .roles
        .update(&role.id, &draft)
        .unwrap();
    let meta = call_meta(&s, &private_bot("caro"))
        .await
        .expect("私人 Bot 的调用带身份元数据");
    assert_eq!(meta.0[TOOL_CALL_USER_META], "caro");
    assert_eq!(meta.0[TOOL_CALL_CAPABILITIES_META], json!([]));
}

#[tokio::test]
async fn shared_bots_call_without_identity_metadata() {
    let s = notes_setup().await;
    let meta = call_meta(&s, &shared_bot()).await.expect("调用送达插件");
    assert!(
        !meta.0.contains_key(TOOL_CALL_USER_META),
        "共享 Bot 不带用户：{meta:?}"
    );
    assert!(
        !meta.0.contains_key(TOOL_CALL_CAPABILITIES_META),
        "共享 Bot 不带能力：{meta:?}"
    );
}

#[tokio::test]
async fn keepers_without_a_member_record_cannot_call() {
    let s = notes_setup().await;
    let result = s
        .call_as(
            &private_bot("nobody"),
            "app-example-notes__search",
            json!({}),
        )
        .await;
    assert_eq!(failure_kind(&result), Some("permission_denied"));
    assert!(s.notes.received_meta.lock().unwrap().is_empty());
}
