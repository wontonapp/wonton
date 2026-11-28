//! 别人的私人 Bot 对家人是 `absent`：列表和详情两个接口一致。

use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::Value;

use crate::gateway::family_tests::send_as;
use crate::gateway::tests::{CORE, Harness, WHOAMI};
use crate::members::MemberRecord;
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, PluginPackage, PluginStatus};

use super::app_with_contact_facts;

#[tokio::test]
async fn family_plugin_responses_mark_others_private_bot_absent() {
    const OWNER_BOT: &str = "@owner-assistant:test";
    let h = Harness::start(None).await;
    let (app, _matrix_state) = app_with_contact_facts(&h).await;
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
    h.manager
        .bot_for_test(&PluginId::parse("app.whoami").unwrap(), OWNER_BOT);
    h.members
        .invite(MemberRecord {
            invited_by: None,
            role_id: "admin".to_owned(),
            wants_private_bot: false,
            ..MemberRecord::for_test("mia")
        })
        .unwrap();
    h.members.mark_joined("mia").unwrap();
    let owner = SecretKey::generate().public();
    h.tunnel.devices.add(owner, None).unwrap();
    let member = SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(member, None, Some("mia".into()), |_| true)
        .unwrap();

    assert_bot_on_both_endpoints(&app, Some(owner), OWNER_BOT, "visible").await;
    assert_bot_on_both_endpoints(&app, Some(member), OWNER_BOT, "absent").await;

    const MEMBER_BOT: &str = "@mia-assistant:test";
    h.manager.bot_records_for_test(
        &PluginId::parse("app.whoami").unwrap(),
        &[(
            "default",
            MEMBER_BOT,
            wonton_protocol::BotKeeper::User {
                username: "mia".into(),
            },
            "test-token",
        )],
    );
    assert_bot_on_both_endpoints(&app, Some(member), MEMBER_BOT, "visible").await;
    assert_bot_on_both_endpoints(&app, Some(owner), MEMBER_BOT, "absent").await;
}

/// 列表和详情两个接口对同一个 Bot 给出的用户 ID 和可见性。
const BOT_ENDPOINTS: [(&str, &str, &str); 2] = [
    (
        "/_wonton/api/plugins",
        "/plugins/0/bot_user_id",
        "/plugins/0/default_bot_visibility",
    ),
    (
        "/_wonton/api/plugins/app.whoami",
        "/installed/bot_user_id",
        "/installed/default_bot_visibility",
    ),
];

/// 两个接口都给出这个 Bot 的 ID，可见性都是 `expected`。
async fn assert_bot_on_both_endpoints(
    app: &axum::Router,
    device: Option<iroh::EndpointId>,
    bot: &str,
    expected: &str,
) {
    for (path, pointer, visibility) in BOT_ENDPOINTS {
        let reply = send_as(app, device, Method::GET, CORE, path, &[], "").await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        assert_eq!(
            reply.json().pointer(pointer).and_then(Value::as_str),
            Some(bot),
            "{path}"
        );
        assert_eq!(
            reply.json().pointer(visibility).and_then(Value::as_str),
            Some(expected),
            "{path}"
        );
    }
}
