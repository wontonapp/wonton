//! 接上真实联邦转发组件的行为测试：状态、最近连上的时刻、同意之后的第一次探活。

use super::*;

/// 刚配好、探活还没有结果：状态是“正在确认”（`Checking`），不报离线；探活一出结果就换成真状态。
#[tokio::test]
async fn a_paired_server_is_checking_until_the_first_probe_result() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Jiale", "b.example.internal").await;
    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    a.request_now("b.example.internal");
    a.approve("b.example.internal").await.unwrap();
    b.wait_paired("a.example.internal").await;

    let (states, rx) = watch::channel(wonton_tunnel::federation::PeerStates::new());
    b.peers.attach_forwarder(rx);
    assert_eq!(
        b.row("a.example.internal").unwrap().status,
        PeerStatus::Checking,
        "还没有探活结果时不报离线"
    );

    states.send_replace(BTreeMap::from([(
        "a.example.internal".to_owned(),
        wonton_tunnel::federation::PeerStatus::Connected,
    )]));
    b.wait_row_status("a.example.internal", PeerStatus::Connected)
        .await;
    b.wait_connected_at("a.example.internal").await;
}

/// 最近连上的时刻跟着状态变化写进记录：对方停掉以后还是那个时刻，重启后还在。
#[tokio::test]
async fn the_last_connected_time_is_kept_after_the_peer_stops() {
    let mut a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Jiale", "b.example.internal").await;
    pair(&a, &b).await;

    let (states, rx) = watch::channel(PeerStates::new());
    a.peers.attach_forwarder(rx);
    states.send_replace(BTreeMap::from([(
        "b.example.internal".to_owned(),
        TunnelPeerStatus::Connected,
    )]));
    let connected_at = a.wait_connected_at("b.example.internal").await;

    states.send_replace(BTreeMap::from([(
        "b.example.internal".to_owned(),
        TunnelPeerStatus::Offline {
            since: std::time::SystemTime::now(),
            last_error: wonton_tunnel::TunnelError::TransportUnavailable {
                reason: "对方停了".to_owned(),
            },
        },
    )]));
    a.wait_row_status("b.example.internal", PeerStatus::Offline)
        .await;
    assert_eq!(
        a.row("b.example.internal").unwrap().last_connected_ms,
        Some(connected_at),
        "离线时要给出最后连上的时刻"
    );

    let (reopened, _) = a.reopen().await;
    let row = reopened
        .view(&langs())
        .peers
        .into_iter()
        .find(|row| row.server_name == "b.example.internal")
        .expect("记录还在");
    assert_eq!(row.status, PeerStatus::Checking, "重启后还没探活");
    assert_eq!(row.last_connected_ms, Some(connected_at), "重启后还在");
}

/// 同意之后的第一次探活就能连上：中间不出“对方服务器拒绝了本机”，扫码方回送的结果也有人收。
#[tokio::test]
async fn the_first_probe_after_approval_connects() {
    let mut a = TestCore::start("Alex", "a.example.internal").await;
    let mut b = TestCore::start("Jiale", "b.example.internal").await;
    a.enable_federation().await;
    b.enable_federation().await;
    let history = a.record_states();

    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    a.request_now("b.example.internal");
    a.approve("b.example.internal").await.unwrap();

    a.wait_row_status("b.example.internal", PeerStatus::Connected)
        .await;

    let seen: Vec<TunnelPeerStatus> = history
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _)| name == "b.example.internal")
        .map(|(_, status)| status.clone())
        .collect();
    assert_eq!(seen.last(), Some(&TunnelPeerStatus::Connected), "{seen:?}");
    assert!(
        seen.iter()
            .all(|status| *status == TunnelPeerStatus::Connected),
        "第一次探活就该连上，中间出现了 {seen:?}"
    );

    // 扫码方把“已同意”送回给出码方：出码方收下，不报警告。
    let reply = deliver_decision(
        &b.peers.tunnel().endpoint,
        a.addr(),
        true,
        b.peers.own_identity(),
    )
    .await;
    assert!(reply.is_ok(), "配对结果应当送到：{reply:?}");
}
