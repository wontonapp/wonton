//! A reset changes identity, not merely the address: pair the new identity without reusing A's data.

use super::*;
use wonton_client::{ConnectionNotice, ServerName};

async fn paired(phone: &Phone, home: &Home) {
    phone.pair(&home.code(None)).unwrap();
    let server_id = home.codes.server_id.to_string();
    phone
        .wait_for(|view| {
            view.status == HomeStatus::Connected
                && view.server_id.as_deref() == Some(server_id.as_str())
                && matches!(view.pairing, Some(PairingStatus::Paired))
        })
        .await;
    tokio::time::timeout(WAIT, async {
        let subscription = phone.subscribe_servers();
        while let Some(view) = subscription.next().await {
            if !view.adding && view.servers.iter().any(|row| row.current) {
                return;
            }
        }
        panic!("server subscription ended");
    })
    .await
    .expect("pairing should be saved");
}

#[tokio::test]
async fn new_identity_at_the_same_address_pairs_without_clearing_old_data() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start().await;
    paired(&phone, &a).await;
    let a_id = a.codes.server_id.to_string();
    let address = a.codes.direct_addrs[0];
    let old_slot = phone.servers().servers[0].id.clone();
    phone
        .dispatch(Action::SetAutoAddApps { enabled: false })
        .unwrap();
    phone
        .dispatch(Action::RenameServer {
            id: old_slot.clone(),
            name: "Home".into(),
        })
        .unwrap();
    let old_tabs = dir
        .path()
        .join("servers")
        .join(&old_slot)
        .join("layout.json");
    let old_data = std::fs::read(&old_tabs).unwrap();
    a.server.shutdown().await;
    let b = Home::start_at(address).await;
    assert_ne!(a_id, b.codes.server_id.to_string());
    assert_eq!(b.codes.direct_addrs, [address]);

    paired(&phone, &b).await;
    let servers = phone.servers();
    assert_eq!(
        servers.servers.len(),
        2,
        "a new endpoint ID is another server"
    );
    assert!(
        !servers
            .servers
            .iter()
            .find(|row| row.id == old_slot)
            .unwrap()
            .current
    );
    assert!(phone.pages().auto_add, "B must not inherit A's settings");
    assert_eq!(std::fs::read(&old_tabs).unwrap(), old_data);
    drop(phone);
    let phone = Phone::open(dir.path());
    phone
        .wait_for(|view| {
            view.status == HomeStatus::Connected
                && view.server_id.as_deref() == Some(&b.codes.server_id.to_string())
        })
        .await;
    assert_eq!(phone.servers(), servers);
    assert!(phone.pages().auto_add);
    phone
        .dispatch(Action::SwitchServer { id: old_slot })
        .unwrap();
    assert!(!phone.pages().auto_add, "A's settings remain usable");
    old_identity_stays_unreachable(&phone, &a_id).await;
}

async fn old_identity_stays_unreachable(phone: &Phone, server_id: &str) {
    let failed = phone
        .wait_for(|view| {
            view.server_id.as_deref() == Some(server_id)
                && matches!(
                    view.notice,
                    Some(
                        ConnectionNotice::Unreachable { .. }
                            | ConnectionNotice::ServerOffline { .. }
                    )
                )
        })
        .await;
    assert!(failed.details.last_failure.is_some());
    phone.dispatch(Action::RetryConnection).unwrap();
    phone
        .wait_for(|view| {
            matches!(
                view.notice,
                Some(
                    ConnectionNotice::Unreachable { retrying: true }
                        | ConnectionNotice::ServerOffline { retrying: true }
                )
            )
        })
        .await;
    tokio::time::timeout(WAIT, async {
        let subscription = phone.subscribe_home();
        while let Some(view) = subscription.next().await {
            if view.details.failed_dials > failed.details.failed_dials
                && matches!(
                    view.notice,
                    Some(
                        ConnectionNotice::Unreachable { retrying: false }
                            | ConnectionNotice::ServerOffline { retrying: false }
                    )
                )
            {
                return;
            }
        }
        panic!("home subscription ended");
    })
    .await
    .expect("manual retry should finish with a truthful failure");
}

#[tokio::test]
async fn pairing_the_same_identity_directly_uses_a_fresh_slot_and_keeps_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start().await;
    paired(&phone, &home).await;
    let old = phone.servers().servers[0].id.clone();
    phone
        .dispatch(Action::RenameServer {
            id: old.clone(),
            name: "Home".into(),
        })
        .unwrap();
    phone
        .dispatch(Action::SetAutoAddApps { enabled: false })
        .unwrap();

    paired(&phone, &home).await;
    let servers = phone.servers();
    assert_eq!(servers.servers.len(), 1);
    assert_ne!(servers.servers[0].id, old);
    assert_eq!(
        servers.servers[0].name,
        ServerName::Named {
            name: "Home".into()
        }
    );
    assert!(phone.pages().auto_add);
    tokio::time::timeout(WAIT, async {
        while dir.path().join("servers").join(&old).exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("same-ID pairing discards the old slot");
}

#[tokio::test]
async fn invalid_pairing_input_does_not_leave_the_configured_server() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start().await;
    paired(&phone, &home).await;
    let before = phone.servers();
    assert!(matches!(
        phone.pair("not a pairing code"),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(matches!(
        phone.dispatch(Action::PairHome {
            code: SecretString::new(home.code(None)),
            device_name: "  ".into(),
        }),
        Err(CoreError::InvalidInput { .. })
    ));
    assert_eq!(phone.servers(), before);
    assert_eq!(phone.home().status, HomeStatus::Connected);
    phone.pair(&home.code(None).replace("v=0", "v=9")).unwrap();
    let (reason, _) = phone.pairing_failure().await;
    assert_eq!(reason, PairingFailure::Protocol);
    assert_eq!(phone.servers(), before);
    assert_eq!(phone.home().status, HomeStatus::Connected);
}

#[tokio::test]
async fn failed_pairing_in_an_added_slot_can_return_to_the_unchanged_server() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start().await;
    paired(&phone, &home).await;
    let before = phone.servers();
    phone
        .dispatch(Action::SetAutoAddApps { enabled: false })
        .unwrap();
    phone.dispatch(Action::AddServer).unwrap();
    phone.pair(&home.code(Some(RejectReason::Expired))).unwrap();
    let (reason, _) = phone.pairing_failure().await;
    assert_eq!(reason, PairingFailure::Expired);
    assert_eq!(phone.servers().servers.len(), 1);
    assert!(phone.servers().adding);
    phone.dispatch(Action::CancelAddServer).unwrap();
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    assert_eq!(phone.servers(), before);
    assert!(!phone.pages().auto_add);
}
