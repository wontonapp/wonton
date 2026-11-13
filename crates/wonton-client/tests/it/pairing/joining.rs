use super::*;
use wonton_protocol::{Permissions, RoleTemplate};

#[tokio::test]
async fn a_joining_code_waits_without_configuring_and_pairs_once_approved() {
    let home = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let role = RoleDef {
        id: "child".to_owned(),
        name: String::new(),
        template: Some(RoleTemplate::Child),
        permissions: Permissions::strictest(),
        people: 0,
    };
    let code = home.join_code(JoinOutcome::Pending, Some(role));
    for _ in 0..2 {
        phone.pair(&code).unwrap();
        let (failure, _) = phone.pairing_failure().await;
        assert_eq!(failure, PairingFailure::PendingApproval);
        assert_eq!(phone.home().status, HomeStatus::NotConfigured);
        assert!(phone.home().server_id.is_none());
        assert!(phone.home().proxy.is_none());
        assert!(phone.servers().servers.is_empty());
    }
    let secret = PairingCode::parse(&code)
        .unwrap()
        .secret
        .expose()
        .to_owned();
    home.codes.joins.lock().unwrap().remove(&secret);
    home.codes.codes.lock().unwrap().insert(secret, None);
    phone.pair(&code).unwrap();
    phone.wait_for(paired).await;
}

#[tokio::test]
async fn denied_and_unknown_join_results_do_not_save_connection_information() {
    let home = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    for (outcome, expected) in [
        (JoinOutcome::Rejected, PairingFailure::JoinRejected),
        (JoinOutcome::CodeUsed, PairingFailure::CodeUsed),
        (JoinOutcome::Other, PairingFailure::PendingApproval),
    ] {
        phone.pair(&home.join_code(outcome, None)).unwrap();
        let (failure, _) = phone.pairing_failure().await;
        assert_eq!(failure, expected);
        assert_eq!(phone.home().status, HomeStatus::NotConfigured);
        assert!(phone.home().server_id.is_none());
        assert!(phone.home().proxy.is_none());
        assert!(phone.servers().servers.is_empty());
    }
    phone.pair(&home.code(None)).unwrap();
    phone.wait_for(paired).await;
}

#[tokio::test]
async fn an_approved_join_response_connects_the_registered_device() {
    let home = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let code = home.join_code(JoinOutcome::Approved, None);
    let id: EndpointId = phone.home().device_id.parse().unwrap();
    home.codes.allowlist.allow(id);
    phone.pair(&code).unwrap();
    phone.wait_for(paired).await;
}
