use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};

use super::app_with_contact_facts;
use crate::gateway::family_tests::{bearer, send_as};
use crate::gateway::tests::{CORE, Harness, WHOAMI};
use crate::members::MemberRecord;
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, PluginPackage, PluginStatus};

/// 装好 whoami 插件的共用底座：contact facts 钉着 mia 的活跃联系人，mia、lee 各一台设备，
/// Bot 记录是主人的私 Bot（`@private:test`）。`_matrix_state` 撑住 Matrix 状态目录的生命周期。
struct PrivateBotFixture {
    h: Harness,
    app: axum::Router,
    _matrix_state: tempfile::TempDir,
    plugin: PluginId,
    other_plugin: PluginId,
    devices: Vec<iroh::EndpointId>,
}

async fn installed_whoami_with_private_bot() -> PrivateBotFixture {
    let h = Harness::start(None).await;
    let contact_path = h.data_dir.join("contact-facts.json");
    std::fs::write(
        contact_path,
        serde_json::to_vec(&json!({"bots": {"app.whoami/default": {
            "bot": {"plugin_id": "app.whoami", "bot": "default"},
            "contacts": {"@mia:test": {"kind": "person", "state": "active", "updated_at_ms": 1}}
        }}}))
        .unwrap(),
    )
    .unwrap();
    let (app, matrix_state) = app_with_contact_facts(&h).await;
    let source = WHOAMI
        .replace("  kind: app", "  kind: agent\n  agent: {protocol: matrix}")
        .replace(
            "  permissions: []",
            "  bot: {display_name: {zh: 助手, en: Assistant}}\n  bots: {services: [web]}\n  permissions: [matrix.bot]",
        );
    h.manager.register_for_test(
        PluginPackage::manifest_of(&source, &Languages::default()).unwrap(),
        PluginStatus::Running,
        None,
    );
    // 第二个 Agent 插件也有自己的默认 Bot：私人 Bot 不能让它的 Bot 从列表里消失。
    h.manager.register_for_test(
        PluginPackage::manifest_of(
            &source.replace("app.whoami", "app.other"),
            &Languages::default(),
        )
        .unwrap(),
        PluginStatus::Running,
        None,
    );
    let plugin = PluginId::parse("app.whoami").unwrap();
    let other_plugin = PluginId::parse("app.other").unwrap();
    let mut devices = Vec::new();
    for name in ["mia", "lee"] {
        h.members
            .invite(MemberRecord {
                role_id: "admin".into(),
                ..MemberRecord::for_test(name)
            })
            .unwrap();
        h.members.mark_joined(name).unwrap();
        let device = SecretKey::generate().public();
        h.tunnel
            .devices
            .add_if(device, None, Some(name.into()), |_| true)
            .unwrap();
        devices.push(device);
    }
    h.manager.bot_records_for_test(
        &plugin,
        &[(
            "default",
            "@private:test",
            wonton_protocol::BotKeeper::User {
                username: "owner".into(),
            },
            "test-token",
        )],
    );
    PrivateBotFixture {
        h,
        app,
        _matrix_state: matrix_state,
        plugin,
        other_plugin,
        devices,
    }
}

#[tokio::test]
async fn default_identity_preserves_active_contacts_and_shared_bots() {
    let PrivateBotFixture {
        h,
        app,
        _matrix_state,
        plugin,
        other_plugin,
        devices,
    } = installed_whoami_with_private_bot().await;
    let path = "/_wonton/api/plugins/app.whoami";
    let contact = send_as(&app, Some(devices[0]), Method::GET, CORE, path, &[], "").await;
    assert_eq!(contact.status, StatusCode::OK, "{}", contact.text());
    assert_eq!(contact.json()["installed"]["bot_user_id"], "@private:test");
    let stranger = send_as(&app, Some(devices[1]), Method::GET, CORE, path, &[], "").await;
    assert_eq!(stranger.status, StatusCode::OK, "{}", stranger.text());
    assert_eq!(
        stranger.json()["installed"]["default_bot_visibility"],
        "absent"
    );
    // 不能用也给出身份：插件页点得开私聊，房间行说为什么。
    assert_eq!(stranger.json()["installed"]["bot_user_id"], "@private:test");

    let token = bearer();
    let as_owner = send_as(
        &app,
        Some(devices[1]),
        Method::GET,
        CORE,
        path,
        &[
            ("authorization", &token),
            ("origin", "https://another.example"),
        ],
        "",
    )
    .await;
    assert_eq!(as_owner.status, StatusCode::OK, "{}", as_owner.text());
    assert_eq!(as_owner.json()["installed"]["bot_user_id"], "@private:test");
    let cross_site = send_as(
        &app,
        Some(devices[1]),
        Method::GET,
        CORE,
        path,
        &[("origin", "https://another.example")],
        "",
    )
    .await;
    assert_eq!(cross_site.status, StatusCode::FORBIDDEN);

    h.manager.bot_records_for_test(&plugin, &[]);
    let missing = send_as(&app, Some(devices[1]), Method::GET, CORE, path, &[], "").await;
    assert_eq!(missing.status, stranger.status, "{}", missing.text());
    // Bot 记录都没了：身份和资格都无从核实。
    assert_eq!(
        missing.json()["installed"]["default_bot_visibility"],
        "unknown"
    );
    assert_eq!(missing.json()["installed"]["bot_user_id"], Value::Null);

    h.manager.bot_records_for_test(
        &plugin,
        &[(
            "default",
            "@shared:test",
            wonton_protocol::BotKeeper::Household,
            "test-token",
        )],
    );
    let shared = send_as(&app, Some(devices[1]), Method::GET, CORE, path, &[], "").await;
    assert_eq!(shared.status, StatusCode::OK, "{}", shared.text());
    assert_eq!(shared.json()["installed"]["bot_user_id"], "@shared:test");
    assert_eq!(
        shared.json()["installed"]["default_bot_visibility"],
        "visible"
    );
}

#[tokio::test]
async fn mismatched_homeserver_is_an_error() {
    let f = installed_whoami_with_private_bot().await;
    assert_mismatched_homeserver_is_an_error(&f.h, &f.app, &f.plugin, f.devices[0]).await;
}

async fn assert_mismatched_homeserver_is_an_error(
    h: &Harness,
    app: &axum::Router,
    plugin: &PluginId,
    device: iroh::EndpointId,
) {
    h.manager.bot_records_for_test(
        plugin,
        &[(
            "default",
            "@private:elsewhere",
            wonton_protocol::BotKeeper::User {
                username: "owner".into(),
            },
            "test-token",
        )],
    );
    let reply = send_as(
        app,
        Some(device),
        Method::GET,
        CORE,
        "/_wonton/api/plugins/app.whoami",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error_kind(), Some("invalid_bot_record"));
    assert!(!reply.text().contains("@private:elsewhere"));
}

/// 有私人 Bot 的家人：默认 Agent 是自己 Bot 所在的那个插件。别的插件和没有私人 Bot 权限的人都用默认 Bot。
/// 有私人 Bot、还在建的人：插件默认 Bot 先顶上，不让用户没 Bot 可用。
#[tokio::test]
async fn a_private_bot_member_defaults_to_their_own_bot() {
    let PrivateBotFixture {
        h,
        app,
        _matrix_state,
        plugin,
        other_plugin,
        devices,
    } = installed_whoami_with_private_bot().await;
    let private = wonton_protocol::BotRef {
        plugin_id: "app.whoami".into(),
        bot: "user-mia".into(),
    };
    h.members.reserve_private_bot("mia", &private).unwrap();
    h.members.private_bot_ready("mia", &private).unwrap();
    h.manager.bot_records_for_test(
        &plugin,
        &[
            (
                "default",
                "@private:test",
                wonton_protocol::BotKeeper::User {
                    username: "owner".into(),
                },
                "owner-token",
            ),
            (
                "user-mia",
                "@mia-bot:test",
                wonton_protocol::BotKeeper::User {
                    username: "mia".into(),
                },
                "mia-token",
            ),
        ],
    );
    let path = "/_wonton/api/plugins/app.whoami";
    let own = send_as(&app, Some(devices[0]), Method::GET, CORE, path, &[], "").await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.text());
    assert_eq!(own.json()["installed"]["bot_user_id"], "@mia-bot:test");
    assert_eq!(own.json()["installed"]["default_bot_visibility"], "visible");

    // 别的插件不被私人 Bot 覆盖：它自己的默认 Bot 仍在。
    h.manager.bot_records_for_test(
        &other_plugin,
        &[(
            "default",
            "@other-bot:test",
            wonton_protocol::BotKeeper::User {
                username: "owner".into(),
            },
            "other-token",
        )],
    );
    let other = send_as(
        &app,
        Some(devices[0]),
        Method::GET,
        CORE,
        "/_wonton/api/plugins/app.other",
        &[],
        "",
    )
    .await;
    assert_eq!(other.status, StatusCode::OK, "{}", other.text());
    assert_eq!(other.json()["installed"]["bot_user_id"], "@other-bot:test");

    // 能看这个应用、但角色里没有私人 Bot 的人：还是插件的默认 Bot。
    let mut permissions = h.roles.get("admin").unwrap().permissions;
    permissions.private_bot = false;
    let role = h
        .roles
        .create(&wonton_protocol::RoleDraft {
            name: "no-privbte-bot".into(),
            permissions,
        })
        .unwrap();
    h.members.set_role("lee", &role.id).unwrap();
    let shared = send_as(&app, Some(devices[1]), Method::GET, CORE, path, &[], "").await;
    assert_eq!(shared.status, StatusCode::OK, "{}", shared.text());
    assert_eq!(shared.json()["installed"]["bot_user_id"], "@private:test");

    // 有 Ready 私人 Bot 的成员被换成没有私人 Bot 权限的角色：即便有记录也退回默认 Bot。
    h.members.set_role("mia", &role.id).unwrap();
    let demoted = send_as(&app, Some(devices[0]), Method::GET, CORE, path, &[], "").await;
    assert_eq!(demoted.status, StatusCode::OK, "{}", demoted.text());
    assert_eq!(demoted.json()["installed"]["bot_user_id"], "@private:test");

    // 自己不想要私人 Bot 的人：有记录也退回插件默认 Bot。
    h.members
        .invite(MemberRecord {
            role_id: "admin".into(),
            wants_private_bot: false,
            ..MemberRecord::for_test("noe")
        })
        .unwrap();
    h.members.mark_joined("noe").unwrap();
    let noe_device = SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(noe_device, None, Some("noe".into()), |_| true)
        .unwrap();
    h.manager.bot_records_for_test(
        &plugin,
        &[
            (
                "default",
                "@private:test",
                wonton_protocol::BotKeeper::User {
                    username: "owner".into(),
                },
                "owner-token",
            ),
            (
                "user-noe",
                "@noe-bot:test",
                wonton_protocol::BotKeeper::User {
                    username: "noe".into(),
                },
                "noe-token",
            ),
        ],
    );
    let unwilling = send_as(&app, Some(noe_device), Method::GET, CORE, path, &[], "").await;
    assert_eq!(unwilling.status, StatusCode::OK, "{}", unwilling.text());
    assert_eq!(
        unwilling.json()["installed"]["bot_user_id"],
        "@private:test"
    );

    // 有私人 Bot、还在建：插件默认 Bot 先顶上，不让用户没 Bot 可用。
    h.members
        .invite(MemberRecord {
            role_id: "admin".into(),
            wants_private_bot: true,
            ..MemberRecord::for_test("amy")
        })
        .unwrap();
    h.members.mark_joined("amy").unwrap();
    let device = SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(device, None, Some("amy".into()), |_| true)
        .unwrap();
    let pending = send_as(&app, Some(device), Method::GET, CORE, path, &[], "").await;
    assert_eq!(pending.status, StatusCode::OK, "{}", pending.text());
    // 私人 Bot 还没建好时退回插件的默认 Bot：能先聊，建好再换成自己的。
    assert_eq!(pending.json()["installed"]["bot_user_id"], "@private:test");
    let _ = h;
}
