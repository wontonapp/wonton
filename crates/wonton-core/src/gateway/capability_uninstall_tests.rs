use axum::body::Body;
use hyper::{Method, StatusCode};
use serde_json::json;

use super::tests::{CORE, Harness, Reply, TOKEN};
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, PluginStatus};

const PROVIDER: &str = "app.capability-provider";
const CONSUMER: &str = "app.capability-consumer";
const CAPABILITY: &str = "x-acme.ledger";

fn package(id: &str, declaration: &str) -> String {
    format!(
        "services:\n  api:\n    image: nginx\nx-wonton:\n  spec: '0.11'\n  id: {id}\n  version: 1.0.0\n  kind: app\n  title: {{en: App}}\n  description: {{en: App}}\n  permissions: []\n{declaration}\n"
    )
}

async fn installed() -> Harness {
    let h = Harness::start(None).await;
    h.manager
        .install(
            &package(
                PROVIDER,
                &format!("  provides:\n    {CAPABILITY}: {{service: api, port: 8080}}"),
            ),
            &[],
            Languages::default(),
        )
        .await
        .unwrap();
    h.manager
        .install(
            &package(CONSUMER, &format!("  requires: [{CAPABILITY}]")),
            &[],
            Languages::default(),
        )
        .await
        .unwrap();
    h
}

async fn remove(h: &Harness, id: &str, body: Body) -> Reply {
    let bearer = format!("Bearer {TOKEN}");
    h.send(
        Method::DELETE,
        CORE,
        &format!("/_wonton/api/plugins/{id}?purge=true"),
        &[
            ("authorization", &bearer),
            ("content-type", "application/json"),
        ],
        body,
    )
    .await
}

fn still_running(h: &Harness, id: &str) {
    assert_eq!(
        h.manager
            .list()
            .iter()
            .find(|plugin| plugin.id.as_str() == id)
            .unwrap()
            .status,
        PluginStatus::Running,
    );
}

#[tokio::test]
async fn capability_uninstall_only_removes_the_unused_providers_explicitly_selected() {
    let h = installed().await;
    let preview = h
        .manager
        .uninstall_preview(&PluginId::parse(CONSUMER).unwrap(), &Languages::default())
        .unwrap();
    assert_eq!(
        preview
            .unused_providers
            .iter()
            .map(|provider| provider.id.as_str())
            .collect::<Vec<_>>(),
        [PROVIDER]
    );
    let blocked = remove(&h, PROVIDER, Body::empty()).await;
    assert_eq!(blocked.status, StatusCode::CONFLICT, "{}", blocked.text());
    assert_eq!(blocked.json()["error"]["kind"], "in_use");
    assert_eq!(blocked.json()["error"]["plugins"][0]["id"], CONSUMER);
    still_running(&h, PROVIDER);
    still_running(&h, CONSUMER);
    let reply = remove(
        &h,
        CONSUMER,
        Body::from(json!({"providers": [PROVIDER]}).to_string()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    for id in [PROVIDER, CONSUMER] {
        assert!(
            !h.manager
                .list()
                .iter()
                .any(|plugin| plugin.id.as_str() == id)
        );
    }
    assert_eq!(reply.json()["containers"].as_array().unwrap().len(), 2);
    assert_eq!(reply.json()["networks"].as_array().unwrap().len(), 2);
    assert!(h.docker.containers().is_empty());
}

#[tokio::test]
async fn capability_uninstall_rejects_invalid_or_newly_used_selections_before_removing_anything() {
    let h = installed().await;
    for selected in [json!([PROVIDER, PROVIDER]), json!(["app.unrelated"])] {
        let before = h.docker.writes();
        let reply = remove(
            &h,
            CONSUMER,
            Body::from(json!({"providers": selected}).to_string()),
        )
        .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text());
        assert_eq!(h.docker.writes(), before);
        still_running(&h, PROVIDER);
        still_running(&h, CONSUMER);
    }
    h.manager
        .install(
            &package("app.another", &format!("  wants: [{CAPABILITY}]")),
            &[],
            Languages::default(),
        )
        .await
        .unwrap();
    let before = h.docker.writes();
    let reply = remove(
        &h,
        CONSUMER,
        Body::from(json!({"providers": [PROVIDER]}).to_string()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text());
    assert_eq!(h.docker.writes(), before);
    let reply = remove(&h, CONSUMER, Body::empty()).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    still_running(&h, PROVIDER);
    still_running(&h, "app.another");
}

async fn replacement_and_member(h: &Harness) -> iroh::EndpointId {
    let source = package(
        "app.replacement",
        &format!("  provides:\n    {CAPABILITY}: {{service: api, port: 8080}}"),
    )
    .replace("  permissions: []", "  permissions: [docker.socket]")
    .replace(
        "    image: nginx",
        "    image: nginx\n    volumes: ['/var/run/docker.sock:/var/run/docker.sock']",
    );
    h.manager
        .install(
            &source,
            &[crate::plugin::manifest::Permission::DockerSocket],
            Languages::default(),
        )
        .await
        .unwrap();
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
    let device = iroh::SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(device, None, Some("mia".into()), |_| true)
        .unwrap();
    device
}

#[tokio::test]
async fn capability_uninstall_by_a_member_does_not_authorize_a_high_privilege_replacement() {
    for required in [true, false] {
        let h = installed().await;
        if !required {
            h.manager
                .uninstall(
                    &PluginId::parse(CONSUMER).unwrap(),
                    false,
                    &Languages::default(),
                )
                .await
                .unwrap();
            h.manager
                .install(
                    &package(CONSUMER, &format!("  wants: [{CAPABILITY}]")),
                    &[],
                    Languages::default(),
                )
                .await
                .unwrap();
        }
        let device = replacement_and_member(&h).await;
        let before = h.docker.writes();
        let reply = super::family_tests::send_as(
            &h.app,
            Some(device),
            Method::DELETE,
            CORE,
            &format!("/_wonton/api/plugins/{PROVIDER}"),
            &[],
            "",
        )
        .await;
        if required {
            assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
            assert_eq!(reply.json()["error"]["kind"], "permission_denied");
            assert_eq!(h.docker.writes(), before);
            still_running(&h, PROVIDER);
            still_running(&h, CONSUMER);
            let owner = remove(&h, PROVIDER, Body::empty()).await;
            assert_eq!(owner.status, StatusCode::OK, "{}", owner.text());
            assert_eq!(
                h.manager
                    .capability_route(
                        &PluginId::parse(CONSUMER).unwrap(),
                        CAPABILITY,
                        &Languages::default(),
                    )
                    .await
                    .unwrap()
                    .provider
                    .as_str(),
                "app.replacement"
            );
        } else {
            assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
            still_running(&h, CONSUMER);
            assert!(matches!(
                h.manager
                    .capability_route(
                        &PluginId::parse(CONSUMER).unwrap(),
                        CAPABILITY,
                        &Languages::default(),
                    )
                    .await,
                Err(crate::plugin::dependencies::CapabilityAccessError::Denied)
            ));
        }
    }
}
