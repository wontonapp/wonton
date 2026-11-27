//! 公网访问关着时，已配对的行报“公网访问已关闭”；私用名、还没探活、对方删了配对各照旧规则。

use std::sync::Arc;

use super::*;
use crate::entry_access::EntryAccess;

async fn pair_with_public_names(a: &TestCore, b: &TestCore) {
    let code = a.peers.issue_code();
    b.scan_code(&code, a).await.unwrap();
    a.request_now("b.example.test");
    a.approve("b.example.test").await.unwrap();
    a.wait_paired("b.example.test").await;
    b.wait_paired("a.example.test").await;
}

fn access(enabled: bool) -> Arc<EntryAccess> {
    let dir = tempfile::tempdir().unwrap();
    let access = Arc::new(EntryAccess::open(dir.path().join("core/entry-access.json")).unwrap());
    if enabled {
        access
            .save(wonton_protocol::PublicAccess { enabled: true })
            .unwrap();
    }
    std::mem::forget(dir);
    access
}

#[tokio::test]
async fn public_access_off_turns_a_connected_row_into_public_access_off() {
    let mut a = TestCore::start("Alex", "a.example.test").await;
    let b = TestCore::start("Alice", "b.example.test").await;
    pair_with_public_names(&a, &b).await;
    a.enable_federation().await;
    a.wait_row_status("b.example.test", PeerStatus::Connected)
        .await;
    // 关掉总开关：行立刻换类别，不用等探活周期。
    a.peers.attach_public_access(access(false));
    assert_eq!(
        a.row("b.example.test").unwrap().status,
        PeerStatus::PublicAccessOff
    );
    // 重新打开：回到转发组件的真实状态。
    a.peers.attach_public_access(access(true));
    assert_eq!(
        a.row("b.example.test").unwrap().status,
        PeerStatus::Connected
    );
}

#[tokio::test]
async fn no_probe_result_stays_checking_while_public_access_is_off() {
    let a = TestCore::start("Alex", "a.example.test").await;
    let b = TestCore::start("Alice", "b.example.test").await;
    pair_with_public_names(&a, &b).await;
    a.peers
        .attach_forwarder(tokio::sync::watch::channel(PeerStates::new()).1);
    a.peers.attach_public_access(access(false));
    assert_eq!(
        a.row("b.example.test").unwrap().status,
        PeerStatus::Checking,
        "还没探活过，不知道经哪条路"
    );
}

#[tokio::test]
async fn private_names_keep_the_forwarder_status_when_public_access_is_off() {
    let mut a = TestCore::start("Alex", "a.example.wonton.internal").await;
    let b = TestCore::start("Alice", "b.example.wonton.internal").await;
    pair_named(
        &a,
        &b,
        "a.example.wonton.internal",
        "b.example.wonton.internal",
    )
    .await;
    a.enable_federation().await;
    a.wait_row_status("b.example.wonton.internal", PeerStatus::Connected)
        .await;
    a.peers.attach_public_access(access(false));
    assert_eq!(
        a.row("b.example.wonton.internal").unwrap().status,
        PeerStatus::Connected
    );
}

#[tokio::test]
async fn an_unpaired_peer_removal_wins_over_public_access_off() {
    let mut a = TestCore::start("Alex", "a.example.test").await;
    let b = TestCore::start("Alice", "b.example.test").await;
    pair_with_public_names(&a, &b).await;
    a.enable_federation().await;
    a.wait_row_status("b.example.test", PeerStatus::Connected)
        .await;
    a.peers.attach_public_access(access(false));
    a.peers.note_removed_by_peer("b.example.test");
    a.wait_row_status("b.example.test", PeerStatus::RemovedByPeer)
        .await;
}
