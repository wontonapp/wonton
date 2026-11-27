//! 证书钉住（ADR-0112）：配对成立记成待钉、证书互换与回执、晚到的证书补发、首次握手的 TOFU、窗口到期，
//! 以及 `tls_verification` 的返回。

use super::*;

/// 配对成立时记成待钉、窗口从当前核心版本起算；证书一到就钉，回执后只留最新一张，删配对时 pin 一起没。
#[tokio::test]
async fn pins_start_pending_and_follow_certificates() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let mut b = TestCore::start("Alice", "b.example.internal").await;
    pair(&a, &b).await;

    let pending = PinState::Pending {
        since_version: crate::build_info::version().to_owned(),
    };
    for core in [&a, &b] {
        let inner = core.peers.lock();
        let record = inner.peers.values().next().unwrap();
        assert_eq!(record.pin_state, pending);
        assert!(record.peer_pin.is_empty());
    }

    let first = certificate("a.example.internal");
    let second = certificate("a.example.internal");
    b.peers.on_certificate(a.id_endpoint(), &first, false);
    b.peers.on_certificate(a.id_endpoint(), &second, false);
    assert_eq!(
        pins(&b.peers, "a.example.internal"),
        vec![first.clone(), second.clone()]
    );
    assert_eq!(
        b.peers.lock().peers["a.example.internal"].pin_state,
        PinState::Pinned
    );

    // 对方回执了我们最新的那张：旧的丢掉。
    b.peers.on_certificate(a.id_endpoint(), &second, true);
    assert_eq!(pins(&b.peers, "a.example.internal"), vec![second.clone()]);

    // 不是已配对的 endpoint 发来的证书不落盘。
    b.peers.on_certificate(
        SecretKey::generate().public(),
        &certificate("other.example.internal"),
        false,
    );
    assert_eq!(pins(&b.peers, "a.example.internal"), vec![second.clone()]);

    // 钉住的证书落盘，重启后还在。
    let (reopened, _) = b.reopen().await;
    assert_eq!(pins(&reopened, "a.example.internal"), vec![second.clone()]);

    // 删配对：记录连着 pin 一起删，文件里也不留。
    reopened.remove("a.example.internal").await.unwrap();
    assert!(!reopened.lock().peers.contains_key("a.example.internal"));
    assert!(
        !std::fs::read_to_string(b.store_path())
            .unwrap()
            .contains("spki_sha256")
    );
}

/// 老版本写的记录没有 pin 相关字段：读成待钉、没有入口，窗口起点补上当前核心版本并落盘。
#[tokio::test]
async fn old_records_read_as_pending_on_the_current_version() {
    let mut a = TestCore::start("Alex", "a.example.internal").await;
    let record = legacy_record(&SecretKey::generate().public().to_string());
    std::fs::write(
        a.store_path(),
        serde_json::json!({"peers": {"old.example.internal": record}}).to_string(),
    )
    .unwrap();

    let (reopened, _) = a.reopen().await;
    let inner = reopened.lock();
    let record = &inner.peers["old.example.internal"];
    assert_eq!(
        record.pin_state,
        PinState::Pending {
            since_version: crate::build_info::version().to_owned(),
        }
    );
    assert!(record.peer_pin.is_empty());
    drop(inner);

    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(a.store_path()).unwrap()).unwrap();
    assert_eq!(
        stored["peers"]["old.example.internal"]["pin_state"]["pending"]["since_version"],
        crate::build_info::version()
    );
}

/// 老版本写的记录形状（没有 peer_pin、pin_state）。
fn legacy_record(endpoint_id: &str) -> serde_json::Value {
    serde_json::json!({
        "server_name": "old.example.internal",
        "endpoint_id": endpoint_id,
        "relays": [],
        "addrs": [],
        "owner_user_id": "@alex:old.example.internal",
        "owner_display_name": "Alex",
        "paired_at_ms": 1,
        "phase": "paired",
    })
}

/// 两边都接了本机证书：配对成立后自动互换、钉住、回执，各留对方一张，状态转成已钉。
#[tokio::test]
async fn pairing_exchanges_certificates() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    let a_certificate = certificate("a.example.internal");
    let b_certificate = certificate("b.example.internal");
    a.set_certificate(a_certificate.clone());
    b.set_certificate(b_certificate.clone());
    pair(&a, &b).await;

    a.wait_for(
        |core| !pins(&core.peers, "b.example.internal").is_empty(),
        "对方证书送到",
    )
    .await;
    b.wait_for(
        |core| !pins(&core.peers, "a.example.internal").is_empty(),
        "对方证书送到",
    )
    .await;
    assert_eq!(pins(&a.peers, "b.example.internal"), vec![b_certificate]);
    assert_eq!(pins(&b.peers, "a.example.internal"), vec![a_certificate]);
    assert_eq!(
        a.peers.lock().peers["b.example.internal"].pin_state,
        PinState::Pinned
    );
    assert_eq!(
        b.peers.lock().peers["a.example.internal"].pin_state,
        PinState::Pinned
    );
}

/// 本机证书接上得晚（配对先成立、安装产物后到）：已经有、还没 pin 的配对现在补发一次。
#[tokio::test]
async fn attaching_the_certificate_late_sends_it_to_an_unpinned_pairing() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    pair(&a, &b).await;
    assert!(pins(&a.peers, "b.example.internal").is_empty());

    let b_certificate = certificate("b.example.internal");
    b.set_certificate(b_certificate.clone());
    assert!(b.peers.own_certificate().is_some());

    a.wait_for(
        |core| !pins(&core.peers, "b.example.internal").is_empty(),
        "补发的证书",
    )
    .await;
    assert_eq!(pins(&a.peers, "b.example.internal"), vec![b_certificate]);
}

/// 探活/连上成功时再送一次：配对后那几次没送到（对方离线）也补得上，不靠运气。
#[tokio::test]
async fn a_connected_probe_resends_a_certificate_that_never_arrived() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    let b_certificate = certificate("b.example.internal");
    b.set_certificate(b_certificate.clone());
    pair(&a, &b).await;
    a.wait_for(
        |core| !pins(&core.peers, "b.example.internal").is_empty(),
        "第一次送达",
    )
    .await;
    // 模拟对方从没收到：清掉 a 手里的 pin，转回待钉。
    {
        let mut inner = a.peers.lock();
        let record = inner.peers.get_mut("b.example.internal").unwrap();
        record.peer_pin.clear();
        record.pin_state = PinState::Pending {
            since_version: "0.8.0".to_owned(),
        };
    }

    let (states, rx) = watch::channel(BTreeMap::new());
    b.peers.attach_forwarder(rx);
    states.send_replace(BTreeMap::from([(
        "a.example.internal".to_owned(),
        TunnelPeerStatus::Connected,
    )]));
    a.wait_for(
        |core| !pins(&core.peers, "b.example.internal").is_empty(),
        "连上后补发",
    )
    .await;
    assert_eq!(pins(&a.peers, "b.example.internal"), vec![b_certificate]);
}

/// 首次握手里看到的证书按升级专用 TOFU 钉住：只对还没有 pin 的已配对记录，钉过就不覆盖。
#[tokio::test]
async fn the_first_handshake_certificate_pins_only_an_unpinned_pairing() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    pair(&a, &b).await;

    let first = certificate("a.example.internal");
    assert!(b.peers.tofu_pin("a.example.internal", &first));
    assert_eq!(pins(&b.peers, "a.example.internal"), vec![first]);
    assert_eq!(
        b.peers.lock().peers["a.example.internal"].pin_state,
        PinState::Pinned
    );

    let second = certificate("a.example.internal");
    assert!(!b.peers.tofu_pin("a.example.internal", &second));
    assert_eq!(pins(&b.peers, "a.example.internal").len(), 1);
}

/// 窗口已到期、还没 pin 的已配对记录如实报出来；没到期、已钉住的都不报。
#[tokio::test]
async fn expired_windows_are_reported_and_pinned_ones_are_not() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    pair(&a, &b).await;
    {
        let mut inner = a.peers.lock();
        let record = inner.peers.get_mut("b.example.internal").unwrap();
        record.pin_state = PinState::Pending {
            since_version: "0.8.0".to_owned(),
        };
    }
    assert!(a.peers.expired_pending_on("0.9.0").is_empty(), "还没到期");
    assert_eq!(a.peers.expired_pending_on("0.10.0"), ["b.example.internal"]);

    a.peers
        .lock()
        .peers
        .get_mut("b.example.internal")
        .unwrap()
        .pin_state = PinState::Pinned;
    assert!(
        a.peers.expired_pending_on("0.10.0").is_empty(),
        "钉住就不报"
    );
}

/// 复审补测：`tls_verification` 要把落下的 pin 读出来交给隧道；否则生产里恒 `None`、永远走待钉。
#[tokio::test]
async fn review_tls_verification_returns_the_pinned_certificate() {
    use wonton_tunnel::federation::PeerVerification;

    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    pair(&a, &b).await;
    let cert = certificate("a.example.internal");
    b.peers.on_certificate(a.id_endpoint(), &cert, false);

    match b.peers.tls_verification("a.example.internal") {
        Some(PeerVerification::Pinned(pins)) => {
            assert_eq!(pins.len(), 1);
            assert_eq!(pins[0].spki_sha256(), cert.spki_sha256_bytes().unwrap());
        }
        other => panic!("有 pin 时应回 Pinned，实际 {other:?}"),
    }
    assert!(
        b.peers
            .tls_verification("unknown.example.internal")
            .is_none(),
        "没配对的查不到"
    );
}

/// 还没 pin 的已配对记录：`tls_verification` 回带窗口信息的待钉，并如实给出窗口起点（开发构建不算到期）。
#[tokio::test]
async fn tls_verification_reports_the_pending_window() {
    use wonton_tunnel::federation::PeerVerification;

    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    pair(&a, &b).await;
    b.peers
        .lock()
        .peers
        .get_mut("a.example.internal")
        .unwrap()
        .pin_state = PinState::Pending {
        since_version: "0.8.0".to_owned(),
    };

    match b.peers.tls_verification("a.example.internal") {
        Some(PeerVerification::Pending {
            since_version,
            expired,
        }) => {
            assert_eq!(since_version.as_deref(), Some("0.8.0"));
            assert!(!expired, "开发构建的版本不可排序，按没到期");
        }
        other => panic!("没 pin 时应回待钉，实际 {other:?}"),
    }
}

/// 钉住状态但 pin 数据坏了：fail-closed 成空根库，不回一条宽一点的待钉。
#[tokio::test]
async fn tls_verification_fails_closed_on_a_broken_pin() {
    use wonton_tunnel::federation::PeerVerification;

    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    pair(&a, &b).await;
    {
        let mut inner = b.peers.lock();
        let record = inner.peers.get_mut("a.example.internal").unwrap();
        record.pin_state = PinState::Pinned;
        record.peer_pin = vec![super::PeerPin {
            certificate: PeerCertificate {
                der: "YmFk".to_owned(),
                spki_sha256: "00".repeat(32),
            },
            pinned_at_ms: 1,
        }];
    }
    match b.peers.tls_verification("a.example.internal") {
        Some(PeerVerification::Pinned(pins)) => assert!(pins.is_empty(), "坏 pin 清空根库"),
        other => panic!("钉住的记录应回 Pinned，实际 {other:?}"),
    }
}
