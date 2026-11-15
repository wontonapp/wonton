use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::json;

use super::app_with_contact_facts;
use crate::gateway::family_tests::{bearer, send_as};
use crate::gateway::tests::{CORE, Harness, WHOAMI};
use crate::members::MemberRecord;
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, PluginPackage, PluginStatus};

#[tokio::test]
async fn default_identity_preserves_active_contacts_and_shared_bots() {
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
    let plugin = PluginId::parse("app.whoami").unwrap();
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
    assert!(!stranger.text().contains("@private:test"));

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
    assert_eq!(
        missing.json()["installed"]["default_bot_visibility"],
        stranger.json()["installed"]["default_bot_visibility"],
    );
    assert_eq!(
        missing.json()["installed"]["bot_user_id"],
        stranger.json()["installed"]["bot_user_id"]
    );

    assert_mismatched_homeserver_is_an_error(&h, &app, &plugin, devices[0]).await;

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
