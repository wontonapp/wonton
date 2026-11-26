use super::*;

async fn paused_home() -> (
    tempfile::TempDir,
    Arc<HomeLink>,
    watch::Receiver<PushMethod>,
) {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::open(dir.path(), dir.path()).unwrap();
    let (_method_tx, method_rx) = watch::channel(PushMethod::default());
    let home = HomeLink::new(
        tokio::runtime::Handle::current(),
        wonton_tunnel::sharing::ProxyHub::bind().await.unwrap(),
        storage,
        Network::DirectOnly,
        None,
        method_rx.clone(),
    )
    .unwrap();
    (dir, home, method_rx)
}

#[tokio::test(start_paused = true)]
async fn a_media_pause_keeps_checking_for_a_minute() {
    let (_dir, home, _method_rx) = paused_home().await;
    home.backgrounded();
    home.page_media_active(true);
    home.page_media_active(false);
    tokio::time::sleep(Duration::from_secs(59)).await;
    assert!(!*home.background.borrow(), "59 秒内还应保活");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(*home.background.borrow(), "过了宽限期回到后台");
}

#[tokio::test(start_paused = true)]
async fn going_to_the_background_during_the_grace_keeps_checking() {
    let (_dir, home, _method_rx) = paused_home().await;
    home.backgrounded();
    home.page_media_active(true);
    home.page_media_active(false);
    tokio::time::sleep(Duration::from_secs(10)).await;
    home.backgrounded();
    tokio::time::sleep(Duration::from_secs(49)).await;
    assert!(!*home.background.borrow(), "宽限期内切后台仍然保活");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(*home.background.borrow(), "宽限期结束才回到后台");
}

#[tokio::test(start_paused = true)]
async fn a_woken_release_does_not_override_playback() {
    let (_dir, home, _method_rx) = paused_home().await;
    home.backgrounded();
    home.page_media_active(true);
    home.page_media_active(false);
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert!(*home.background.borrow(), "宽限期先结束");
    home.page_media_active(true);
    tokio::time::sleep(Duration::from_secs(90)).await;
    assert!(!*home.background.borrow(), "旧释放任务不再把保活关掉");
}

#[tokio::test(start_paused = true)]
async fn playing_again_cancels_the_release() {
    let (_dir, home, _method_rx) = paused_home().await;
    home.backgrounded();
    home.page_media_active(true);
    home.page_media_active(false);
    tokio::time::advance(Duration::from_secs(30)).await;
    home.page_media_active(true);
    tokio::time::advance(Duration::from_secs(90)).await;
    tokio::task::yield_now().await;
    assert!(!*home.background.borrow());
}

#[tokio::test(start_paused = true)]
async fn page_media_keeps_the_tunnel_checked_in_the_background() {
    let (_dir, home, _method_rx) = paused_home().await;
    home.backgrounded();
    assert!(*home.background.borrow());
    home.page_media_active(true);
    assert!(!*home.background.borrow());
    home.page_media_active(false);
    assert!(!*home.background.borrow(), "宽限期内仍然保活");
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert!(*home.background.borrow());
    home.foregrounded();
    assert!(!*home.background.borrow());
    home.backgrounded();
    assert!(*home.background.borrow());
    home.page_media_active(true);
    home.foregrounded();
    home.backgrounded();
    assert!(!*home.background.borrow());
}

/// 核心自己经隧道请求的闸门：只拦已知的传输不通；连接中、已连上、还没配对、被拒绝都不拦。
#[tokio::test(start_paused = true)]
async fn a_known_unreachable_connection_gates_core_requests() {
    let (_dir, home, _method_rx) = paused_home().await;
    assert_eq!(home.tunnel_outage(), None, "还没配对不拦");
    for status in [HomeStatus::Connecting, HomeStatus::Connected] {
        home.view.send_modify(|view| view.status = status.clone());
        assert_eq!(home.tunnel_outage(), None, "{status:?} 不拦");
    }
    let down = |reason| HomeStatus::Disconnected {
        reason,
        message: "测试".into(),
    };
    home.view
        .send_modify(|view| view.status = down(HomeFailure::Unauthorized));
    assert_eq!(
        home.tunnel_outage(),
        None,
        "被拒绝不拦：请求是放行后恢复的路"
    );
    home.view
        .send_modify(|view| view.status = down(HomeFailure::TransportUnavailable));
    assert!(home.tunnel_outage().is_some(), "传输不通要拦");
}

#[test]
fn the_path_switch_is_off_by_default_and_roundtrips() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!path_settings::exclude_fake_ip(dir.path()));
    path_settings::set_exclude_fake_ip(dir.path(), true).unwrap();
    assert!(path_settings::exclude_fake_ip(dir.path()));
    path_settings::set_exclude_fake_ip(dir.path(), false).unwrap();
    assert!(!path_settings::exclude_fake_ip(dir.path()));
}

#[test]
fn a_broken_path_setting_file_reads_as_off() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("path-settings.json"), "not json").unwrap();
    assert!(!path_settings::exclude_fake_ip(dir.path()));
}

/// 开关两种取值下都能真的绑起来：装上选路器不能把绑定本身弄坏。
#[tokio::test]
async fn the_endpoint_binds_with_the_path_switch_off_and_on() {
    let (_dir, home, _method_rx) = paused_home().await;
    let deadline = Duration::from_secs(10);
    let off = home
        .bind_endpoint(&[], 1, crate::l10n::language(), deadline)
        .await;
    assert!(off.is_ok(), "关着能绑：{:?}", off.err());
    home.set_fake_ip_exclusion(true).unwrap();
    assert!(home.fake_ip_exclusion.load(Ordering::SeqCst), "缓存已打开");
    let on = home
        .bind_endpoint(&[], 1, crate::l10n::language(), deadline)
        .await;
    assert!(on.is_ok(), "开着能绑：{:?}", on.err());
}

/// 写失败时缓存和文件都保持原值。
#[tokio::test]
async fn a_failed_write_keeps_the_old_value() {
    let (dir, home, _method_rx) = paused_home().await;
    home.set_fake_ip_exclusion(true).unwrap();
    // 设置文件的位置放成目录，下一次写改名必失败。
    let path = dir.path().join("path-settings.json");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(home.set_fake_ip_exclusion(false).is_err());
    assert!(
        home.fake_ip_exclusion.load(Ordering::SeqCst),
        "写失败时缓存保持原值"
    );
}

/// 一连上直连路径就写盘，不等 `/_wonton/api/tunnel` 的报告：推送唤醒的进程常常活不到报告回来，
/// 那一次学到的地址不能因此丢掉（ADR-0004 的“上次连上的地址”）。
#[tokio::test]
async fn a_direct_connect_is_saved_without_waiting_for_the_report() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::open(dir.path(), dir.path()).unwrap();
    let (_method_tx, method_rx) = watch::channel(PushMethod::default());
    let home = HomeLink::new(
        tokio::runtime::Handle::current(),
        wonton_tunnel::sharing::ProxyHub::bind().await.unwrap(),
        storage,
        Network::DirectOnly,
        None,
        method_rx,
    )
    .unwrap();
    let device = home.view().device_id.parse().unwrap();
    let server = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .bind_addr((std::net::Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let addr = server.bound_sockets()[0];
    let upstream = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let upstream = upstream.local_addr().unwrap();
    let tunnel = wonton_tunnel::TunnelServer::spawn(
        server.clone(),
        wonton_tunnel::Allowlist::new([device]),
        upstream,
    );
    let text = format!(
        r#"{{"server": {{"endpoint_id": "{}", "direct_addrs": ["{addr}"]}},
            "base_domain": "wonton.localhost"}}"#,
        server.id()
    );
    home.connect(&text).unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    let saved = loop {
        if home.view().status == HomeStatus::Connected {
            let text = std::fs::read_to_string(dir.path().join("home-ticket.json")).unwrap();
            if text.contains("used_addrs") {
                break text;
            }
        }
        assert!(Instant::now() < deadline, "20 秒内没连上并写盘");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let ticket = Ticket::parse(&saved).unwrap();
    // 端点可能同时绑了 IPv4 和通配 IPv6，iroh 先挑中哪条、报哪个本机地址是它的事：写盘的直连地址得落在
    // 这台端点绑过的 socket 上（通配地址换成这台机器的任一地址都算），不能是猜的。
    assert!(!ticket.used_addrs().is_empty(), "连上后要把直连路径写盘");
    let bound = server.bound_sockets();
    let is_bound = |used: &std::net::SocketAddr| {
        bound.iter().any(|socket| {
            socket.port() == used.port()
                && (socket.ip().is_unspecified() || socket.ip() == used.ip())
        })
    };
    assert!(
        ticket.used_addrs().iter().all(is_bound),
        "记下的直连地址要在这台端点绑过的 socket 上：{:?} vs {:?}",
        ticket.used_addrs(),
        bound
    );
    assert!(ticket.server_addr().ip_addrs().any(|tried| *tried == addr));
    tunnel.shutdown().await;
}

/// 设置页的读失败挂在 `load_problem` 上（还没配对），不留一个空转的 Loading。
#[tokio::test(start_paused = true)]
async fn a_refresh_without_a_paired_server_reports_why() {
    let (_dir, home, _method_rx) = paused_home().await;
    home.relay_settings(wonton_protocol::RelaySettingsAction::Refresh)
        .unwrap();
    let view = home.view();
    assert!(matches!(
        view.relays.phase,
        Some(wonton_protocol::RelaySettingsPhase::Failed { .. })
    ));
    assert!(view.relays.load_problem.is_some());
}

/// 加、改、删在本机立刻生效并标成待保存；没配对时存不了，改动留着。
#[tokio::test(start_paused = true)]
async fn relay_edits_stay_local_when_the_table_cannot_be_saved_yet() {
    let (_dir, home, _method_rx) = paused_home().await;
    home.relay_settings(wonton_protocol::RelaySettingsAction::Add {
        url: "https://relay.example.org".into(),
        token: None,
        quic_port: None,
        tier: None,
    })
    .unwrap();
    home.relay_settings(wonton_protocol::RelaySettingsAction::SetN0 { on: true })
        .unwrap();
    let view = home.view();
    assert_eq!(view.relays.entries.len(), 1);
    assert!(view.relays.n0);
    assert!(view.relays.dirty);
    assert_eq!(
        view.relays.phase,
        Some(wonton_protocol::RelaySettingsPhase::Ready)
    );

    // 没有连接信息：保存当场失败，改动和待保存的标记都还在。
    assert!(matches!(
        home.relay_settings(wonton_protocol::RelaySettingsAction::Save),
        Err(CoreError::NotReady { .. })
    ));
    let view = home.view();
    assert_eq!(view.relays.entries.len(), 1);
    assert!(view.relays.dirty);

    // 丢弃回到核心最近给的那一份：一份都没拿到过就是空表，n0 回落到缺省（留）。
    home.relay_settings(wonton_protocol::RelaySettingsAction::Discard)
        .unwrap();
    let view = home.view();
    assert!(view.relays.entries.is_empty());
    assert!(view.relays.n0);
    assert!(!view.relays.dirty);
}

/// 中继路径带着中继地址，直连路径不带。
#[test]
fn only_a_relay_path_carries_the_relay_url() {
    let url: iroh::RelayUrl = "https://relay.example.org".parse().unwrap();
    let relayed = path_view(PathInfo {
        connection_id: 1,
        kind: PathKind::Relay,
        remote: TransportAddr::Relay(url.clone()),
        rtt: Duration::from_millis(42),
        sent_packets: 0,
        lost_packets: 0,
        sent_bytes: 0,
        lost_bytes: 0,
    });
    assert_eq!(
        relayed.relay_url.as_deref(),
        Some(wonton_tunnel::relays::relay_url_text(&url).as_str())
    );
    assert!(relayed.relay);

    let direct = path_view(PathInfo {
        connection_id: 2,
        kind: PathKind::Direct,
        remote: TransportAddr::Ip("127.0.0.1:1".parse().unwrap()),
        rtt: Duration::from_millis(1),
        sent_packets: 0,
        lost_packets: 0,
        sent_bytes: 0,
        lost_bytes: 0,
    });
    assert_eq!(direct.relay_url, None);
    assert!(!direct.relay);
}
