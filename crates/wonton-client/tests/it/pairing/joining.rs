use super::support::*;
use wonton_protocol::{PermissionItem, Permissions, RoleTemplate};

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
        let view = phone
            .wait_for(|view| matches!(view.pairing, Some(PairingStatus::JoinPending { .. })))
            .await;
        let Some(PairingStatus::JoinPending { role: Some(role) }) = view.pairing else {
            panic!("the server sends the role with a pending join")
        };
        assert_eq!(role.template, Some(RoleTemplate::Child));
        // 最严的角色也允许“只读”，加入页照样写得出这一条。
        assert_eq!(role.abilities, [PermissionItem::Ai]);
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
    ] {
        phone.pair(&home.join_code(outcome, None)).unwrap();
        let (failure, _) = phone.pairing_failure().await;
        assert_eq!(failure, expected);
        assert_eq!(phone.home().status, HomeStatus::NotConfigured);
        assert!(phone.home().server_id.is_none());
        assert!(phone.home().proxy.is_none());
        assert!(phone.servers().servers.is_empty());
    }
    // 比这个 App 新的服务端给的结果按“等批”处理，不编成失败。
    phone
        .pair(&home.join_code(JoinOutcome::Other, None))
        .unwrap();
    phone
        .wait_for(|view| matches!(view.pairing, Some(PairingStatus::JoinPending { .. })))
        .await;
    assert_eq!(phone.home().status, HomeStatus::NotConfigured);
    assert!(phone.servers().servers.is_empty());
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
