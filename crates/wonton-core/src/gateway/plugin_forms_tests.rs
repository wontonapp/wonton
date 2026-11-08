use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hyper::{Method, StatusCode};

use super::family_tests::{bearer, send_as};
use super::tests::{CORE, Harness, WHOAMI};
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, PluginPackage};

fn package(runtime: &str) -> String {
    format!(
        "x-wonton:\n  spec: '0.10'\n  id: app.forms\n  version: 2.0.0\n  kind: app\n  \
         runtime: {runtime}\n  title: {{en: Forms}}\n  description: {{en: Forms}}\n  permissions: []\n  \
         secrets:\n    api_key:\n      input:\n        title: {{en: API key}}\n"
    )
}

fn assert_no_new_resources(h: &Harness, id: &PluginId) {
    assert!(h.manager.record_for_test(id).is_none());
    assert!(h.manager.store().secrets().load(id).unwrap().is_none());
    assert!(h.docker.containers().is_empty());
    assert!(h.docker.networks().is_empty());
    assert!(h.docker.volumes().is_empty());
    assert!(!h.data_dir.join("apps/app-forms").exists());
}

#[tokio::test]
async fn non_compose_uploads_fail_before_storing_inputs_or_installing() {
    let h = Harness::start(None).await;
    let id = PluginId::parse("app.forms").unwrap();
    let secrets = URL_SAFE_NO_PAD.encode(br#"{"api_key":"should-not-be-stored"}"#);
    for runtime in ["skills", "remote_mcp"] {
        let reply = send_as(
            &h.app,
            None,
            Method::POST,
            CORE,
            "/_wonton/api/plugins",
            &[
                ("authorization", bearer().as_str()),
                ("wonton-plugin-secrets", &secrets),
            ],
            &package(runtime),
        )
        .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text());
        assert_eq!(reply.error_kind(), Some("unsupported"));
        assert_eq!(reply.json()["error"]["kind"], "unsupported");
        assert!(
            reply.json()["error"]["message"]
                .as_str()
                .unwrap()
                .contains(runtime)
        );
        assert!(
            h.manager
                .installed_plugin(&id, &Languages::default())
                .is_none()
        );
        assert_no_new_resources(&h, &id);
    }
}

#[tokio::test]
async fn non_compose_upgrades_do_not_touch_the_installed_version() {
    let h = Harness::start(None).await;
    let id = PluginId::parse("app.forms").unwrap();
    let old = "services:\n  web:\n    image: nginx\nx-wonton:\n  spec: '0.6'\n  id: app.forms\n  version: 1.0.0\n  kind: app\n  title: {en: Forms}\n  description: {en: Forms}\n  permissions: []\n";
    h.manager
        .install(old, &[], Languages::default())
        .await
        .unwrap();
    let containers = h.docker.containers();
    let networks = h.docker.networks();
    let volumes = h.docker.volumes();
    for runtime in ["skills", "remote_mcp"] {
        let reply = send_as(
            &h.app,
            None,
            Method::POST,
            CORE,
            "/_wonton/api/plugins/app.forms/upgrade",
            &[("authorization", bearer().as_str())],
            &package(runtime),
        )
        .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text());
        assert_eq!(reply.error_kind(), Some("unsupported"));
        assert_eq!(h.manager.record_for_test(&id).unwrap().version, "1.0.0");
        assert_eq!(
            h.manager
                .installed_plugin(&id, &Languages::default())
                .unwrap()
                .version,
            "1.0.0"
        );
        assert!(h.manager.store().secrets().load(&id).unwrap().is_none());
        assert_eq!(h.docker.containers(), containers);
        assert_eq!(h.docker.networks(), networks);
        assert_eq!(h.docker.volumes(), volumes);
    }
}

#[tokio::test]
async fn only_the_owner_sees_the_installed_scope() {
    let index = tempfile::tempdir().unwrap();
    let compose = WHOAMI
        .replace("  icon: icon.svg\n", "")
        .replace("  webui:\n    service: web\n    port: 8000\n", "");
    let package = PluginPackage::parse(&compose, &Languages::default()).unwrap();
    crate::plugin::storefront::index::IndexEntry::of(
        &package,
        "github:example/whoami",
        &"a".repeat(40),
    )
    .unwrap()
    .write_to(index.path(), &package)
    .unwrap();
    let h = Harness::start_with_store(
        None,
        crate::plugin::storefront::StoreConfig {
            index_dir: Some(index.path().to_owned()),
            community: false,
        },
    )
    .await;
    let device = iroh::SecretKey::generate().public();
    h.members
        .invite(crate::members::MemberRecord {
            username: "mia".into(),
            user_id: "@mia:test".into(),
            display_name: "Mia".into(),
            email: None,
            status: wonton_protocol::MemberStatus::Invited,
            can_install_plugins: true,
            wants_private_bot: false,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        })
        .unwrap();
    h.tunnel
        .devices
        .add_if(device, None, Some("mia".into()), |_| true)
        .unwrap();
    for path in [
        "/_wonton/api/plugins/app.whoami",
        "/_wonton/api/store/plugin?source=github:example/whoami",
    ] {
        let owner = send_as(
            &h.app,
            None,
            Method::GET,
            CORE,
            path,
            &[("authorization", bearer().as_str())],
            "",
        )
        .await;
        assert_eq!(owner.status, StatusCode::OK, "{}", owner.text());
        assert_eq!(owner.json()["runtime"], "compose");
        assert_eq!(owner.json()["installed"]["id"], "app.whoami");
        assert_eq!(owner.json()["scope"]["type"], "all");
        let member = send_as(&h.app, Some(device), Method::GET, CORE, path, &[], "").await;
        assert_eq!(member.status, StatusCode::OK, "{}", member.text());
        assert_eq!(member.json()["installed"]["id"], "app.whoami");
        assert!(
            member.json()["scope"].is_null(),
            "{path}: {}",
            member.text()
        );
    }
}

pub(super) fn owner_only_routes() -> Vec<(Method, &'static str, &'static str)> {
    vec![
        (
            Method::PUT,
            "/_wonton/api/plugins/app.whoami/scope",
            "{\"type\":\"all\"}",
        ),
        (
            Method::PUT,
            "/_wonton/api/bots/app.agent/default/plugin-access/app.whoami",
            "{\"enabled\":false}",
        ),
    ]
}

#[tokio::test]
async fn scope_routes_parse_requests_but_never_report_success_or_change_state() {
    let h = Harness::start(None).await;
    let id = PluginId::parse("app.whoami").unwrap();
    let before = h
        .manager
        .installed_detail(&id, &Languages::default())
        .unwrap();
    for (path, body) in [
        (
            wonton_protocol::plugin_scope_path("app.whoami"),
            "{\"type\":\"all\"}",
        ),
        (
            wonton_protocol::plugin_scope_path("app.whoami"),
            "{\"type\":\"bots\",\"bots\":[{\"plugin_id\":\"app.agent\",\"bot\":\"default\"}]}",
        ),
        (
            wonton_protocol::bot_plugin_access_path("app.agent", "default", "app.whoami"),
            "{\"enabled\":false}",
        ),
        (
            wonton_protocol::bot_plugin_access_path("app.agent", "default", "app.whoami"),
            "{\"enabled\":true}",
        ),
    ] {
        let reply = send_as(
            &h.app,
            None,
            Method::PUT,
            CORE,
            &path,
            &[
                ("authorization", bearer().as_str()),
                ("content-type", "application/json"),
            ],
            body,
        )
        .await;
        assert_eq!(
            reply.status,
            StatusCode::NOT_IMPLEMENTED,
            "{}",
            reply.text()
        );
        assert_eq!(reply.error_kind(), Some("unsupported"));
        assert_eq!(reply.json()["error"]["kind"], "unsupported");
    }
    assert_eq!(
        h.manager
            .installed_detail(&id, &Languages::default())
            .unwrap(),
        before
    );
    assert_no_new_resources(&h, &PluginId::parse("app.agent").unwrap());
}

#[tokio::test]
async fn scope_routes_reject_invalid_request_shapes() {
    let h = Harness::start(None).await;
    for (path, body) in [
        (wonton_protocol::plugin_scope_path("app.whoami"), "{}"),
        (
            wonton_protocol::plugin_scope_path("app.whoami"),
            "{\"type\":\"bots\",\"bots\":false}",
        ),
        (
            wonton_protocol::plugin_scope_path("invalid"),
            "{\"type\":\"all\"}",
        ),
        (
            wonton_protocol::bot_plugin_access_path("app.agent", "default", "app.whoami"),
            "{\"enabled\":\"false\"}",
        ),
        (
            wonton_protocol::bot_plugin_access_path("app.agent", "Bad_Name", "app.whoami"),
            "{\"enabled\":false}",
        ),
    ] {
        let reply = send_as(
            &h.app,
            None,
            Method::PUT,
            CORE,
            &path,
            &[
                ("authorization", bearer().as_str()),
                ("content-type", "application/json"),
            ],
            body,
        )
        .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text());
        assert_eq!(reply.error_kind(), Some("invalid_input"));
    }
}

#[tokio::test]
async fn the_plugin_list_carries_declared_tunnel_ports() {
    let h = Harness::start(None).await;
    let tunnel = "services:\n  web:\n    image: nginx\n  media:\n    image: nginx\nx-wonton:\n  spec: '0.11'\n  id: app.remote\n  version: 1.0.0\n  kind: app\n  title: {en: Remote}\n  description: {en: Remote}\n  permissions: []\n  tunnel:\n    ports:\n      - {service: web, port: 47989}\n      - {service: media, port: 47998, protocol: udp}\n";
    h.manager
        .install(tunnel, &[], Languages::default())
        .await
        .unwrap();
    let reply = send_as(
        &h.app,
        None,
        Method::GET,
        CORE,
        "/_wonton/api/plugins",
        &[("authorization", bearer().as_str())],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let plugins = reply.json()["plugins"].as_array().unwrap().clone();
    let ports = |id: &str| {
        plugins
            .iter()
            .find(|plugin| plugin["id"] == id)
            .unwrap_or_else(|| panic!("{id} missing"))["tunnel_ports"]
            .clone()
    };
    assert_eq!(
        ports("app.remote"),
        serde_json::json!([
            {"port": 47989, "protocol": "tcp"},
            {"port": 47998, "protocol": "udp"},
        ])
    );
    assert_eq!(ports("app.whoami"), serde_json::json!([]));
}
