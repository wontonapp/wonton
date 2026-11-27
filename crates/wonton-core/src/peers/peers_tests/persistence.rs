//! 重启和记录不过期：请求和等对方的记录跨重启留着，重启后自己问一次结果，待处理的行照旧列出。

use super::*;

/// 请求和等对方的记录都不过期：重启后都还在。
#[tokio::test]
async fn a_restart_keeps_pending_requests_and_pairings() {
    let mut a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;

    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    a.request_now("b.example.internal");

    let (reopened, _) = a.reopen().await;
    assert_eq!(reopened.view(&langs()).requests.len(), 1);
    assert_eq!(
        reopened.view(&langs()).requests[0].server_name,
        "b.example.internal"
    );
}

/// 扫码之后核心重启：记录留着，启动时自己问一次结果，对方同意后就成立。
#[tokio::test]
async fn a_pending_scan_is_polled_again_after_a_restart() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let mut b = TestCore::start("Alice", "b.example.internal").await;

    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    a.request_now("b.example.internal");

    // 核心重启（同一份记录），主人这时才同意。
    let (reopened, names) = b.reopen().await;
    a.approve("b.example.internal").await.unwrap();
    a.wait_paired("b.example.internal").await;
    let mut changes = reopened.changes();
    tokio::time::timeout(WAIT, async {
        loop {
            if reopened.lookup("a.example.internal").is_some() {
                return;
            }
            if changes.changed().await.is_err() {
                return;
            }
        }
    })
    .await
    .expect("重启后要自己问出结果");
    let mut names = names;
    tokio::time::timeout(WAIT, async {
        loop {
            if names.borrow_and_update().contains("a.example.internal") {
                return;
            }
            if names.changed().await.is_err() {
                return;
            }
        }
    })
    .await
    .expect("结果送到以后要开始探活");
}

/// 等对方的记录不过期，视图照旧列出。
#[tokio::test]
async fn pending_rows_do_not_expire() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.json");
    let waiting = super::PeerRecord {
        server_name: "late.example.internal".to_owned(),
        endpoint_id: SecretKey::generate().public(),
        relays: Vec::new(),
        addrs: Vec::new(),
        owner_user_id: "@owner:late.example.internal".to_owned(),
        owner_display_name: "Late".to_owned(),
        paired_at_ms: 0,
        phase: super::Phase::AwaitingPeer,
        last_connected_ms: None,
        peer_pin: Vec::new(),
        pin_state: PinState::default(),
    };
    std::fs::write(
        &path,
        serde_json::to_string(&super::Stored {
            peers: BTreeMap::from([(waiting.server_name.clone(), waiting)]),
            ..Default::default()
        })
        .unwrap(),
    )
    .unwrap();
    let devices = Arc::new(Devices::open(dir.path().join("devices.json")).unwrap());
    let endpoint = loopback_endpoint(SecretKey::generate()).await;
    let members = Arc::new(Members::open(dir.path().join("members.json")).unwrap());
    let tunnel = Tunnel::new(
        devices,
        members,
        Arc::new(|| Languages::from_header(None)),
        endpoint,
        0,
        Arc::new(crate::relay_settings::RelaySettings::for_table(
            RelayTable::builtin_n0(),
        )),
        watch::channel(BTreeSet::from(["127.0.0.1".parse().unwrap()])).1,
    );
    let peers = Peers::open(
        &path,
        OwnIdentity {
            server_name: "a.example.internal".to_owned(),
            owner_user_id: "@owner:a.example.internal".to_owned(),
        },
        tunnel,
        None,
        Arc::new(|| "Alex".to_owned()),
        Arc::new(|| Languages::from_header(Some("zh"))),
    )
    .unwrap();
    let row = peers
        .view(&langs())
        .peers
        .into_iter()
        .find(|row| row.server_name == "late.example.internal")
        .expect("还在等的记录照旧列出");
    assert_eq!(row.status, PeerStatus::AwaitingApproval);
}
