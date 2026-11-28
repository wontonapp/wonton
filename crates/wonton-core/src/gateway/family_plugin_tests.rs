//! 家人和插件：按角色开放应用（ADR-0088）、确认卡片归属、OIDC 声明按当前人（ADR-0045）。
//! 共用的搭建在 [`super::family_tests`]。

use std::sync::Arc;

use hyper::{Method, StatusCode};
use iroh::SecretKey;

use super::family_tests::{PORT, send_as};
use super::tests::{CORE, Harness, TOKEN};
use crate::members::MemberRecord;

#[path = "family_bot_projection_race_tests.rs"]
mod family_bot_projection_race_tests;
#[path = "family_bot_projection_tests.rs"]
mod family_bot_projection_tests;

mod private_bots;

pub(super) async fn app_with_contact_facts(h: &Harness) -> (axum::Router, tempfile::TempDir) {
    use super::bot_contacts_tests::{Offline, Recorded};
    use crate::contacts::wiring::PluginDirectory;
    use crate::contacts::{Contacts, Ports};

    let (_, config) = crate::matrix::fake::start().await;
    let matrix_state = tempfile::tempdir().unwrap();
    let matrix = Arc::new(
        crate::matrix::Matrix::connect(&config, matrix_state.path(), h.http.clone())
            .await
            .unwrap(),
    );
    let owner = crate::owner::OwnerStore::for_test(&h.data_dir, &super::tests::owner());
    let receipts = Arc::new(crate::contacts::ApplyReceipts::default());
    let contacts = Contacts::open(
        h.data_dir.join("contact-facts.json"),
        matrix.server_name(),
        Ports {
            directory: Arc::new(PluginDirectory::new(
                h.manager.clone(),
                h.members.clone(),
                owner.clone(),
            )),
            link: Arc::new(Offline),
            invites: Arc::new(Offline),
            effects: Arc::new(Recorded::new(receipts.clone())),
            receipts,
            open_federation: false,
        },
    )
    .unwrap();
    let gateway = super::Gateway::new(
        crate::config::PublicUrls {
            scheme: "http".into(),
            base_domain: CORE.into(),
            port: PORT,
        },
        owner,
        TOKEN.into(),
        h.members.clone(),
        h.manager.clone(),
        None,
        h.http.clone(),
        h.tunnel.clone(),
    )
    .with_roles(h.roles.clone())
    .with_matrix(Some(matrix))
    .with_contacts(Some(contacts));
    (super::router(Arc::new(gateway)), matrix_state)
}

#[tokio::test]
async fn invalid_plugin_ids_are_not_reported_as_missing_bots() {
    let h = Harness::start(None).await;
    let owner = SecretKey::generate().public();
    h.tunnel.devices.add(owner, None).unwrap();
    for (method, path) in [
        (Method::GET, "/_wonton/api/plugins/Bad.Id"),
        (Method::POST, "/_wonton/api/plugins/Bad.Id/start"),
        (Method::POST, "/_wonton/api/plugins/Bad.Id/stop"),
        (Method::POST, "/_wonton/api/plugins/Bad.Id/upgrade"),
        (
            Method::PUT,
            "/_wonton/api/plugins/Bad.Id/permissions/network.egress",
        ),
        (Method::PUT, "/_wonton/api/plugins/Bad.Id/lan-webui"),
        (Method::PUT, "/_wonton/api/plugins/Bad.Id/limits"),
        (Method::PUT, "/_wonton/api/plugins/Bad.Id/ask-every-update"),
        (
            Method::DELETE,
            "/_wonton/api/plugins/Bad.Id/save-grants?folder=photos",
        ),
        (Method::PUT, "/_wonton/api/plugins/Bad.Id/secrets/token"),
    ] {
        let reply = send_as(
            &h.app,
            Some(owner),
            method,
            CORE,
            path,
            &[("content-type", "application/json")],
            "{}",
        )
        .await;
        assert_eq!(
            reply.status,
            StatusCode::BAD_REQUEST,
            "{path}: {}",
            reply.text()
        );
        assert_eq!(reply.error_kind(), Some("invalid_input"), "{path}");
    }
}

/// 记一个“mia”：能不能装插件按测试要的给角色。
async fn invite_record(h: &Harness, can_install: bool) {
    h.members
        .invite(MemberRecord {
            invited_by: None,
            username: "mia".to_owned(),
            user_id: "@mia:test".to_owned(),
            display_name: "Mia".to_owned(),
            email: None,
            status: wonton_protocol::MemberStatus::Invited,
            role_id: if can_install { "admin" } else { "friend" }.to_owned(),
            wants_private_bot: true,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        })
        .unwrap();
}
