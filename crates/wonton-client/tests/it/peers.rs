//! “其他服务器”页面（ADR-0050）的行为测试：四个操作经隧道打到假核心的 `/_wonton/api/peers`，
//! 设置首页的“其他服务器”一行跟着列表出现；对面是本机假扮的核心管理接口。

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::http::{Method, StatusCode};
use wonton_client::{
    Action, PeerCode, PeerError, PeerRow, PeerStatus, PeersState, PeersViewModel, SecretString,
    SettingsEntry,
};
use wonton_protocol::{
    PEER_ERROR_HEADER, PEERS_PATH, PeerScan, PeersView, peer_code_path, peer_path,
};

use crate::plugins::rig::{Canned, Headered, Rig, WAIT};

/// 安静窗：要跨过完整的轮询周期（间隔是客户端核心里的 3 秒常量）。核心跑在自己的运行时里，
/// 测试的暂停时钟驱动不了它，只能观察真实时间（同 tabs/polling.rs 的 `QUIET`）。
const QUIET: Duration = Duration::from_secs(8);
/// 已配对服务器的状态按探活周期（30 秒）取：等一个变化要跨过整整一个周期再加余量。
const STATUS_WAIT: Duration = Duration::from_secs(45);

fn peers_view(peers: Vec<PeerRow>, code: Option<PeerCode>) -> String {
    serde_json::to_string(&PeersView {
        peers,
        requests: Vec::new(),
        rejected: Vec::new(),
        code,
    })
    .unwrap()
}

fn row(server_name: &str) -> PeerRow {
    PeerRow {
        server_name: server_name.to_owned(),
        owner_user_id: format!("@alex:{server_name}"),
        owner_display_name: "Alex".into(),
        status: PeerStatus::Connected,
        last_connected_ms: None,
        detail: None,
    }
}

/// 家里的服务器的错误回应：类别在响应头和错误体里（说明按 App 的语言写好，这里用测试里的占位句）。
fn peer_error_headered(status: u16, kind: &str) -> Headered {
    Headered(
        status,
        vec![(PEER_ERROR_HEADER.to_owned(), kind.to_owned())],
        serde_json::json!({"error": {"kind": kind, "message": "核心写的说明"}}).to_string(),
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

async fn wait_peers(rig: &Rig, done: impl FnMut(&PeersViewModel) -> bool) -> PeersViewModel {
    wait_peers_within(rig, WAIT, done).await
}

async fn wait_peers_within(
    rig: &Rig,
    within: Duration,
    mut done: impl FnMut(&PeersViewModel) -> bool,
) -> PeersViewModel {
    let deadline = Instant::now() + within;
    loop {
        let view = rig.app.peers();
        if done(&view) {
            return view;
        }
        assert!(Instant::now() < deadline, "{view:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_failed(rig: &Rig, error: PeerError) -> PeersViewModel {
    wait_peers(
        rig,
        |view| matches!(&view.state, PeersState::Failed { error: got, .. } if *got == error),
    )
    .await
}

/// 设置首页里的“其他服务器”一行；还没有时为空。
fn peers_entry(rig: &Rig) -> Option<SettingsEntry> {
    rig.app
        .settings()
        .sections
        .iter()
        .flat_map(|section| section.entries.iter().cloned())
        .find(|entry| matches!(entry, SettingsEntry::Peers { .. }))
}

/// 等那一行变成期望的样子。设置首页是跟着 peers 的 watch 另算的，比页面状态晚一步，读完页面直接断言会抢跑。
async fn wait_peers_entry(rig: &Rig, expected: Option<SettingsEntry>) {
    wait_peers_entry_within(rig, WAIT, expected).await;
}

async fn wait_peers_entry_within(rig: &Rig, within: Duration, expected: Option<SettingsEntry>) {
    let deadline = Instant::now() + within;
    loop {
        if peers_entry(rig) == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "设置首页的“其他服务器”一行没有变成 {expected:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// 连上后自动取一次列表（设置首页的台数要它），等它落地。
async fn connected_and_listed(rig: &Rig, peers: Vec<PeerRow>) {
    rig.core.answer_always(
        Method::GET,
        PEERS_PATH,
        Canned(StatusCode::OK.as_u16(), peers_view(peers, None)),
    );
    rig.connected().await;
    wait_peers(rig, |view| view.state == PeersState::Ready).await;
}

/// 刷新得到列表；设置首页的那一行跟着列表出现，台数是已配对的数量。
#[tokio::test]
async fn refreshing_lists_the_paired_servers_and_the_settings_entry() {
    let rig = Rig::new().await;
    connected_and_listed(&rig, vec![row("abc12345.wonton"), row("def67890.wonton")]).await;
    let view = rig.app.peers();
    assert_eq!(view.peers.len(), 2);
    assert_eq!(view.code, None);
    wait_peers_entry(&rig, Some(SettingsEntry::Peers { count: 2 })).await;

    // 下拉刷新：列表换成最新的一份。
    rig.core.answer(
        Method::GET,
        PEERS_PATH,
        Canned(
            StatusCode::OK.as_u16(),
            peers_view(vec![row("abc12345.wonton")], None),
        ),
    );
    rig.app.dispatch(Action::RefreshPeers).unwrap();
    wait_peers(&rig, |view| {
        view.state == PeersState::Ready && view.peers.len() == 1
    })
    .await;
    wait_peers_entry(&rig, Some(SettingsEntry::Peers { count: 1 })).await;
}

/// 出码后有码；到过期时间就清掉，壳不再画。
#[tokio::test]
async fn the_code_shows_until_it_expires() {
    let rig = Rig::new().await;
    connected_and_listed(&rig, Vec::new()).await;

    let code = PeerCode {
        code: SecretString::new("wonton://peer?k=one-time-secret"),
        expires_at_ms: now_ms() + 900,
    };
    rig.core.answer(
        Method::POST,
        &peer_code_path(),
        Canned(
            StatusCode::OK.as_u16(),
            serde_json::to_string(&code).unwrap(),
        ),
    );
    rig.app.dispatch(Action::ShowPeerCode).unwrap();
    let view = wait_peers(&rig, |view| view.code.is_some()).await;
    assert_eq!(view.code.unwrap().expires_at_ms, code.expires_at_ms);
    assert_eq!(rig.core.seen(&Method::POST, &peer_code_path()).len(), 1);

    wait_peers(&rig, |view| view.code.is_none()).await;
}

/// 扫码成功：码原样交给核心，列表刷新后多出“等对方同意”的一条。
#[tokio::test]
async fn scanning_a_code_submits_it_and_refreshes_the_list() {
    let rig = Rig::new().await;
    connected_and_listed(&rig, Vec::new()).await;

    let code = SecretString::new("wonton://peer?k=one-time-secret");
    rig.core.answer(
        Method::POST,
        PEERS_PATH,
        Canned(StatusCode::OK.as_u16(), "{}".into()),
    );
    rig.core.answer(
        Method::GET,
        PEERS_PATH,
        Canned(
            StatusCode::OK.as_u16(),
            peers_view(
                vec![PeerRow {
                    status: PeerStatus::AwaitingApproval,
                    ..row("abc12345.wonton")
                }],
                None,
            ),
        ),
    );
    rig.app
        .dispatch(Action::ScanPeerCode { code: code.clone() })
        .unwrap();
    let view = wait_peers(&rig, |view| {
        view.state == PeersState::Ready && !view.peers.is_empty()
    })
    .await;
    assert_eq!(view.peers[0].status, PeerStatus::AwaitingApproval);
    let seen = rig.core.seen(&Method::POST, PEERS_PATH);
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].body,
        serde_json::to_string(&PeerScan {
            code: SecretString::new("wonton://peer?k=one-time-secret"),
        })
        .unwrap()
    );
}

/// 删除成功后列表刷新：那一条没了，设置首页的台数跟着变。
#[tokio::test]
async fn removing_a_peer_refreshes_the_list() {
    let rig = Rig::new().await;
    connected_and_listed(&rig, vec![row("abc12345.wonton"), row("def67890.wonton")]).await;

    rig.core.answer(
        Method::DELETE,
        &peer_path("abc12345.wonton"),
        Canned(StatusCode::OK.as_u16(), "{}".into()),
    );
    rig.core.answer(
        Method::GET,
        PEERS_PATH,
        Canned(
            StatusCode::OK.as_u16(),
            peers_view(vec![row("def67890.wonton")], None),
        ),
    );
    rig.app
        .dispatch(Action::RemovePeer {
            server_name: "abc12345.wonton".into(),
        })
        .unwrap();
    wait_peers(&rig, |view| {
        view.state == PeersState::Ready && view.peers.len() == 1
    })
    .await;
    wait_peers_entry(&rig, Some(SettingsEntry::Peers { count: 1 })).await;
}

/// 删掉最后一台：列表空掉，设置首页的“其他服务器”是 0，不是删掉之前那 1。
#[tokio::test]
async fn removing_the_last_peer_zeroes_the_settings_count() {
    let rig = Rig::new().await;
    connected_and_listed(&rig, vec![row("abc12345.wonton")]).await;
    wait_peers_entry(&rig, Some(SettingsEntry::Peers { count: 1 })).await;

    rig.core.answer(
        Method::DELETE,
        &peer_path("abc12345.wonton"),
        Canned(StatusCode::OK.as_u16(), "{}".into()),
    );
    rig.core.answer_always(
        Method::GET,
        PEERS_PATH,
        Canned(StatusCode::OK.as_u16(), peers_view(Vec::new(), None)),
    );
    // 取列表压住：那一行要在删除成功后马上没有，不等下一次取列表。
    rig.core.hold(Method::GET, PEERS_PATH);
    rig.app
        .dispatch(Action::RemovePeer {
            server_name: "abc12345.wonton".into(),
        })
        .unwrap();
    wait_peers(&rig, |view| view.peers.is_empty()).await;
    rig.core.release(Method::GET, PEERS_PATH);
    wait_peers(&rig, |view| view.state == PeersState::Ready).await;
    wait_peers_entry(&rig, Some(SettingsEntry::Peers { count: 0 })).await;
}

/// 页面开着、没有别的东西可等：对方掉线在一个探活周期左右显示出来，不用退出页面再进；
/// 对方随后删了这台（列表里没有了），设置首页的台数跟着变。
#[tokio::test]
async fn a_status_change_shows_up_within_a_probe_cycle() {
    let rig = Rig::new().await;
    connected_and_listed(&rig, vec![row("abc12345.wonton")]).await;
    wait_peers_entry(&rig, Some(SettingsEntry::Peers { count: 1 })).await;

    rig.core.answer_always(
        Method::GET,
        PEERS_PATH,
        Canned(
            StatusCode::OK.as_u16(),
            peers_view(
                vec![PeerRow {
                    status: PeerStatus::Offline,
                    ..row("abc12345.wonton")
                }],
                None,
            ),
        ),
    );
    let view = wait_peers_within(&rig, STATUS_WAIT, |view| {
        view.state == PeersState::Ready
            && view
                .peers
                .first()
                .is_some_and(|row| row.status == PeerStatus::Offline)
    })
    .await;
    assert_eq!(view.peers.len(), 1);

    rig.core.answer_always(
        Method::GET,
        PEERS_PATH,
        Canned(StatusCode::OK.as_u16(), peers_view(Vec::new(), None)),
    );
    wait_peers_within(&rig, STATUS_WAIT, |view| {
        view.state == PeersState::Ready && view.peers.is_empty()
    })
    .await;
    wait_peers_entry_within(&rig, STATUS_WAIT, Some(SettingsEntry::Peers { count: 0 })).await;
}

/// 扫码失败：家里的服务器给的类别（`X-Wonton-Error`）落进 `Failed`。
#[tokio::test]
async fn a_failed_scan_fails_with_the_server_s_category() {
    for (kind, status, expected) in [
        ("code_expired", 410, PeerError::CodeExpired),
        ("peer_rejected", 403, PeerError::PeerRejected),
        ("self_pairing", 400, PeerError::SelfPairing),
        ("already_paired", 409, PeerError::AlreadyPaired),
    ] {
        let rig = Rig::new().await;
        connected_and_listed(&rig, Vec::new()).await;
        rig.core
            .answer_headered(Method::POST, PEERS_PATH, peer_error_headered(status, kind));
        rig.app
            .dispatch(Action::ScanPeerCode {
                code: SecretString::new("wonton://peer?k=one-time-secret"),
            })
            .unwrap();
        assert!(
            matches!(
                &wait_failed(&rig, expected).await.state,
                PeersState::Failed { error, .. } if *error == expected
            ),
            "要落在 {expected:?}"
        );
    }
}

/// 比这个 App 新的核心给的类别落到 `Other`，不吞掉失败。
#[tokio::test]
async fn an_unknown_category_fails_with_other() {
    let rig = Rig::new().await;
    connected_and_listed(&rig, Vec::new()).await;
    rig.core.answer_headered(
        Method::POST,
        PEERS_PATH,
        peer_error_headered(400, "code_lost"),
    );
    rig.app
        .dispatch(Action::ScanPeerCode {
            code: SecretString::new("wonton://peer?k=one-time-secret"),
        })
        .unwrap();
    assert!(
        matches!(
            &wait_failed(&rig, PeerError::Other).await.state,
            PeersState::Failed {
                error: PeerError::Other,
                ..
            }
        ),
        "不认识的类别要落到 Other"
    );
}

/// 还没连上家里的服务器时取列表：按“连不上”失败，不是读不懂。
#[tokio::test]
async fn without_a_connection_the_list_fails_with_transport_unavailable() {
    let rig = Rig::new().await;
    rig.app.dispatch(Action::RefreshPeers).unwrap();
    let view = wait_failed(&rig, PeerError::TransportUnavailable).await;
    assert!(matches!(&view.state, PeersState::Failed { message, .. } if !message.is_empty()));
    assert_eq!(peers_entry(&rig), None);
}

/// 隧道不通（本地代理回 502）：按“连不上”失败。
#[tokio::test]
async fn a_broken_tunnel_fails_with_transport_unavailable() {
    let rig = Rig::new().await;
    connected_and_listed(&rig, Vec::new()).await;
    rig.core.answer_headered(
        Method::GET,
        PEERS_PATH,
        Headered(
            502,
            vec![(
                "x-wonton-tunnel-error".into(),
                "transport-unavailable".into(),
            )],
            "隧道未连接\n".into(),
        ),
    );
    rig.app.dispatch(Action::RefreshPeers).unwrap();
    wait_failed(&rig, PeerError::TransportUnavailable).await;
    // 台数拿不到，设置首页不放这一行；刚才是 Ready，要等设置首页跟着撤掉。
    wait_peers_entry(&rig, None).await;
}

/// 读不懂的回应（不是 JSON）：按“读不懂”失败。
#[tokio::test]
async fn an_unreadable_response_fails_with_protocol() {
    let rig = Rig::new().await;
    rig.core
        .answer_always(Method::GET, PEERS_PATH, Canned(200, "不是 JSON".into()));
    rig.connected().await;
    wait_failed(&rig, PeerError::Protocol).await;
    assert_eq!(peers_entry(&rig), None);
}

/// 核心只让主人设备读配对接口：非主人的设备取不到列表，设置首页也没有这一行。
#[tokio::test]
async fn a_device_the_core_does_not_allow_sees_no_peers_entry() {
    let rig = Rig::new().await;
    rig.core.answer_always_headered(
        Method::GET,
        PEERS_PATH,
        peer_error_headered(401, "unauthenticated"),
    );
    rig.connected().await;
    wait_failed(&rig, PeerError::Unauthenticated).await;
    assert_eq!(peers_entry(&rig), None);
}

/// 等待中的轮询：对方扫了码、核心把码作废以后，几秒内 ViewModel 的码也清掉（壳会跟着关弹层）。
/// 没人可等之后就停：再跨过完整的轮询周期也不发列表请求。
#[tokio::test]
async fn a_voided_code_is_picked_up_by_the_poll_and_polling_then_stops() {
    let rig = Rig::new().await;
    connected_and_listed(&rig, Vec::new()).await;

    let code = PeerCode {
        code: SecretString::new("wonton://peer?k=one-time-secret"),
        expires_at_ms: now_ms() + 600_000,
    };
    rig.core.answer(
        Method::POST,
        &peer_code_path(),
        Canned(
            StatusCode::OK.as_u16(),
            serde_json::to_string(&code).unwrap(),
        ),
    );
    rig.app.dispatch(Action::ShowPeerCode).unwrap();
    wait_peers(&rig, |view| view.code.is_some()).await;

    // 假核心这边码已经作废（对方扫掉了），出码的一方也没有可等的一条：下一次轮询把它清掉。
    let view = wait_peers(&rig, |view| view.code.is_none()).await;
    assert_eq!(view.state, PeersState::Ready);

    let before = rig.core.seen(&Method::GET, PEERS_PATH).len();
    let deadline = Instant::now() + QUIET;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        rig.core.seen(&Method::GET, PEERS_PATH).len(),
        before,
        "没人可等了还在发列表请求"
    );
}

/// 扫码后对方点了同意：假核心把那条改成已连上，几秒内 ViewModel 跟上，不用退出页面再进。
#[tokio::test]
async fn an_approval_is_picked_up_by_the_poll() {
    let rig = Rig::new().await;
    connected_and_listed(&rig, Vec::new()).await;

    rig.core.answer(
        Method::POST,
        PEERS_PATH,
        Canned(StatusCode::OK.as_u16(), "{}".into()),
    );
    rig.core.answer_always(
        Method::GET,
        PEERS_PATH,
        Canned(
            StatusCode::OK.as_u16(),
            peers_view(
                vec![PeerRow {
                    status: PeerStatus::AwaitingApproval,
                    ..row("abc12345.wonton")
                }],
                None,
            ),
        ),
    );
    rig.app
        .dispatch(Action::ScanPeerCode {
            code: SecretString::new("wonton://peer?k=one-time-secret"),
        })
        .unwrap();
    wait_peers(&rig, |view| {
        view.state == PeersState::Ready
            && view
                .peers
                .first()
                .is_some_and(|row| row.status == PeerStatus::AwaitingApproval)
    })
    .await;

    // 对方点了同意：下一次轮询取到改过的那行。
    rig.core.answer_always(
        Method::GET,
        PEERS_PATH,
        Canned(
            StatusCode::OK.as_u16(),
            peers_view(vec![row("abc12345.wonton")], None),
        ),
    );
    let view = wait_peers(&rig, |view| {
        view.state == PeersState::Ready
            && view
                .peers
                .first()
                .is_some_and(|row| row.status == PeerStatus::Connected)
    })
    .await;
    assert_eq!(view.peers.len(), 1);
}

/// 刚配对的一行先是「正在确认」（Checking）：假核心把它改成真状态后几秒内 ViewModel 跟上，
/// 之后没人可等，轮询停。
#[tokio::test]
async fn a_checking_row_is_followed_until_it_settles_and_polling_then_stops() {
    let rig = Rig::new().await;
    connected_and_listed(&rig, Vec::new()).await;

    rig.core.answer_always(
        Method::GET,
        PEERS_PATH,
        Canned(
            StatusCode::OK.as_u16(),
            peers_view(
                vec![PeerRow {
                    status: PeerStatus::Checking,
                    ..row("abc12345.wonton")
                }],
                None,
            ),
        ),
    );
    rig.app.dispatch(Action::RefreshPeers).unwrap();
    wait_peers(&rig, |view| {
        view.state == PeersState::Ready
            && view
                .peers
                .first()
                .is_some_and(|row| row.status == PeerStatus::Checking)
    })
    .await;

    // 第一次探活出了结果：下一次轮询取到改过的那行。
    rig.core.answer_always(
        Method::GET,
        PEERS_PATH,
        Canned(
            StatusCode::OK.as_u16(),
            peers_view(vec![row("abc12345.wonton")], None),
        ),
    );
    let view = wait_peers(&rig, |view| {
        view.state == PeersState::Ready
            && view
                .peers
                .first()
                .is_some_and(|row| row.status == PeerStatus::Connected)
    })
    .await;
    assert_eq!(view.peers.len(), 1);

    // 变成真状态就没人可等：跨过完整的轮询周期再没有一个列表请求。
    let before = rig.core.seen(&Method::GET, PEERS_PATH).len();
    let deadline = Instant::now() + QUIET;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        rig.core.seen(&Method::GET, PEERS_PATH).len(),
        before,
        "没人可等了还在发列表请求"
    );
}

/// 没有可等的东西（码没有、列表里没有还没批的请求）时不发轮询请求。
#[tokio::test]
async fn with_nothing_to_wait_for_no_poll_requests_are_made() {
    let rig = Rig::new().await;
    connected_and_listed(&rig, vec![row("abc12345.wonton")]).await;

    let before = rig.core.seen(&Method::GET, PEERS_PATH).len();
    let deadline = Instant::now() + QUIET;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        rig.core.seen(&Method::GET, PEERS_PATH).len(),
        before,
        "没有可等的东西还在发列表请求"
    );
}
