//! 删除配对的行为测试：对方删了以后留着记录、标成对方已断开，只有主人移除才真删；重新配对能替换掉它。

use super::*;

/// 对方删了：本机记录留着，状态是“对方已断开连接”，最近连上的时刻保留；名单撤掉、查找不再认它，
/// 状态写进存储。
#[tokio::test]
async fn a_peer_removal_keeps_the_record_as_removed() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;
    pair(&a, &b).await;

    // 转发组件报过“已连上”：最近连上的时刻记在这一行上。
    let (states, rx) = watch::channel(PeerStates::new());
    a.peers.attach_forwarder(rx);
    states.send_replace(BTreeMap::from([(
        "b.example.internal".to_owned(),
        TunnelPeerStatus::Connected,
    )]));
    let connected_at = a.wait_connected_at("b.example.internal").await;

    b.peers.remove("a.example.internal").await.unwrap();
    a.wait_removed("b.example.internal").await;

    // 对方删了：本机不再拿它当配对过的服务器，但记录还在，时间保留。
    assert!(a.peers.lookup("b.example.internal").is_none());
    assert!(!a.paired_names.borrow().contains("b.example.internal"));
    assert!(!a.peers.server_allowlist().contains(&b.id_endpoint()));
    let row = a.row("b.example.internal").expect("记录要留着");
    assert_eq!(row.last_connected_ms, Some(connected_at));
    // 重启后还是这个状态：已经落盘。
    let stored = std::fs::read_to_string(a.store_path()).unwrap();
    assert!(
        stored.contains("\"phase\": \"removed_by_peer\""),
        "状态要写进存储：{stored}"
    );
}

/// 本机离线时对方删了、上线后握手被对方拒绝：和收到通知是同一个结果，不需要通知送到。
#[tokio::test]
async fn a_handshake_rejection_marks_the_record_like_a_notification() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;
    pair(&a, &b).await;

    // b 删了，通知没送到 a：只有 a 的转发组件探活时被 b 的服务器名单拒绝。
    let (states, rx) = watch::channel(PeerStates::new());
    a.peers.attach_forwarder(rx);
    states.send_replace(BTreeMap::from([(
        "b.example.internal".to_owned(),
        TunnelPeerStatus::RemovedByPeer,
    )]));
    a.wait_removed("b.example.internal").await;

    assert!(a.row("b.example.internal").is_some());
    assert!(a.peers.lookup("b.example.internal").is_none());
    assert!(!a.paired_names.borrow().contains("b.example.internal"));
    let stored = std::fs::read_to_string(a.store_path()).unwrap();
    assert!(
        stored.contains("\"phase\": \"removed_by_peer\""),
        "{stored}"
    );
}

/// 主人对“对方已断开连接”的一行点移除才真删记录；对方已经没有记录了，通知不会报错。
#[tokio::test]
async fn the_owner_removes_a_removed_peer_for_good() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;
    pair(&a, &b).await;

    a.peers.remove("b.example.internal").await.unwrap();
    b.wait_removed("a.example.internal").await;

    // b 的主人把这一行移除：记录真删。
    assert!(b.peers.remove("a.example.internal").await.unwrap());
    b.wait_row_gone("a.example.internal").await;
    assert!(b.peers.view(&langs()).peers.is_empty());
    assert!(!b.peers.remove("a.example.internal").await.unwrap());
}

/// 重启：标记过的记录原样读回，状态仍是“对方已断开连接”，名单和查找里没有它。
#[tokio::test]
async fn a_removed_peer_stays_removed_after_a_restart() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;
    pair(&a, &b).await;
    b.peers.remove("a.example.internal").await.unwrap();
    a.wait_removed("b.example.internal").await;

    let (reopened, names) = a.reopen().await;
    let row = reopened
        .view(&langs())
        .peers
        .into_iter()
        .find(|row| row.server_name == "b.example.internal")
        .expect("重启后记录还在");
    assert_eq!(row.status, PeerStatus::RemovedByPeer);
    assert!(reopened.lookup("b.example.internal").is_none());
    assert!(!names.borrow().contains("b.example.internal"));
}

/// 对方删了以后重新配：对方扫本机的新码，原来的记录被新配对替换，不挡配对。
#[tokio::test]
async fn a_removed_peer_can_pair_again_by_scanning_our_code() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;
    pair(&a, &b).await;
    a.peers.remove("b.example.internal").await.unwrap();
    b.wait_removed("a.example.internal").await;
    b.peers.remove("a.example.internal").await.unwrap();

    let code = a.peers.issue_code();
    b.peers.scan(code.code.expose()).await.unwrap();
    let card = a.cards.wait_for(2).await;
    a.confirmations
        .answer(&card.id, Answer::Approve, None)
        .unwrap();

    a.wait_paired("b.example.internal").await;
    assert_eq!(
        a.row("b.example.internal").unwrap().status,
        PeerStatus::Checking,
        "新配对成立，不再是对方已断开"
    );
    a.wait_paired_names("b.example.internal").await;
    assert!(a.peers.server_allowlist().contains(&b.id_endpoint()));
}

/// 对方删了以后重新配：本机扫对方的新码，原来的记录被新配对替换，不报“已经配对过”。
#[tokio::test]
async fn a_removed_peer_can_pair_again_by_scanning_it() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;
    pair(&a, &b).await;
    b.peers.remove("a.example.internal").await.unwrap();
    a.wait_removed("b.example.internal").await;

    let code = b.peers.issue_code();
    a.peers.scan(code.code.expose()).await.unwrap();
    let row = a.row("b.example.internal").unwrap();
    assert_eq!(row.status, PeerStatus::AwaitingApproval);

    let card = b.cards.wait_for(1).await;
    b.confirmations
        .answer(&card.id, Answer::Approve, None)
        .unwrap();
    a.wait_paired("b.example.internal").await;
    assert_eq!(
        a.row("b.example.internal").unwrap().status,
        PeerStatus::Checking,
        "新配对成立，不再是对方已断开"
    );
}
