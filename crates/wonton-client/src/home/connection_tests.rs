use super::*;

const DOWN: HomeStatus = HomeStatus::Disconnected {
    reason: HomeFailure::TransportUnavailable,
    message: String::new(),
};

fn rejected() -> HomeStatus {
    HomeStatus::Disconnected {
        reason: HomeFailure::Unauthorized,
        message: String::new(),
    }
}

/// 从 `start` 开始断开，`seconds` 秒、客户端报 `failures` 次建连失败之后的提示。
fn after(
    tracker: &mut Tracker,
    start: Instant,
    seconds: u64,
    failures: u32,
) -> Option<ConnectionNotice> {
    let now = start + Duration::from_secs(seconds);
    tracker.observe(&DOWN, tracker.relay, failures, 0, None, now);
    tracker.notice(&DOWN, now)
}

#[test]
fn a_blip_shows_nothing_and_a_long_outage_says_the_server_seems_offline() {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    tracker.observe(&DOWN, ConnectionCheck::Ok, 0, 0, None, start);
    assert_eq!(after(&mut tracker, start, 1, 0), None);
    assert_eq!(
        after(&mut tracker, start, 3, 0),
        Some(ConnectionNotice::Connecting)
    );
    assert_eq!(
        after(&mut tracker, start, 29, 0),
        Some(ConnectionNotice::Connecting)
    );
    assert_eq!(
        after(&mut tracker, start, 30, 0),
        Some(ConnectionNotice::ServerOffline { retrying: false })
    );
}

#[test]
fn a_long_outage_without_the_relay_does_not_blame_the_server() {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    tracker.observe(&DOWN, ConnectionCheck::Failing, 0, 0, None, start);
    assert_eq!(
        after(&mut tracker, start, 30, 0),
        Some(ConnectionNotice::Unreachable { retrying: false })
    );
}

#[test]
fn the_first_failed_dial_shows_after_the_grace() {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    tracker.observe(&DOWN, ConnectionCheck::Failing, 1, 0, None, start);

    // 宽限在前：换网时建连立即失败很常见，0.5 秒还不能闪“连不上”。
    let half = start + Duration::from_millis(500);
    tracker.observe(&DOWN, ConnectionCheck::Failing, 1, 0, None, half);
    assert_eq!(tracker.notice(&DOWN, half), None);
    assert_eq!(after(&mut tracker, start, 1, 1), None);
    assert_eq!(
        after(&mut tracker, start, 3, 1),
        Some(ConnectionNotice::Unreachable { retrying: false })
    );

    // 连得上中继时，怪家里的服务器。
    let mut tracker = Tracker::default();
    tracker.observe(&DOWN, ConnectionCheck::Ok, 1, 0, None, start);
    assert_eq!(
        after(&mut tracker, start, 3, 1),
        Some(ConnectionNotice::ServerOffline { retrying: false })
    );
}

#[test]
fn a_new_round_does_not_inherit_the_previous_failures() {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    tracker.observe(&DOWN, ConnectionCheck::Ok, 1, 0, None, start);
    assert_eq!(
        after(&mut tracker, start, 3, 1),
        Some(ConnectionNotice::ServerOffline { retrying: false })
    );

    // 连上了又断开：新一轮还没失败过，先是宽限，然后才是“连接中”。
    tracker.observe(
        &HomeStatus::Connected,
        ConnectionCheck::Ok,
        0,
        0,
        None,
        start + Duration::from_secs(10),
    );
    let again = start + Duration::from_secs(20);
    tracker.observe(&DOWN, ConnectionCheck::Ok, 0, 0, None, again);
    assert_eq!(tracker.notice(&DOWN, again), None);
    assert_eq!(
        after(&mut tracker, again, 3, 0),
        Some(ConnectionNotice::Connecting)
    );

    // 这一轮再失败，马上说连不上。
    assert_eq!(
        after(&mut tracker, again, 4, 1),
        Some(ConnectionNotice::ServerOffline { retrying: false })
    );
}

#[test]
fn a_relay_blip_does_not_change_the_sentence() {
    let start = Instant::now();
    let at = |seconds| start + Duration::from_secs(seconds);
    let mut tracker = Tracker::default();
    let mut relay_says = |relay, seconds| {
        tracker.observe(&DOWN, relay, 0, 0, None, at(seconds));
        tracker.notice(&DOWN, at(seconds))
    };
    let offline = Some(ConnectionNotice::ServerOffline { retrying: false });
    relay_says(ConnectionCheck::Ok, 0);
    assert_eq!(relay_says(ConnectionCheck::Failing, 31), offline);
    assert_eq!(relay_says(ConnectionCheck::Failing, 35), offline);
    assert_eq!(relay_says(ConnectionCheck::Ok, 36), offline);
    assert_eq!(relay_says(ConnectionCheck::Failing, 40), offline);
    assert_eq!(relay_says(ConnectionCheck::Failing, 49), offline);
    assert_eq!(
        relay_says(ConnectionCheck::Failing, 50),
        Some(ConnectionNotice::Unreachable { retrying: false })
    );
}

#[test]
fn no_network_says_so_however_long_it_lasts() {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    tracker.network_changed(DeviceNetwork::None, start);
    tracker.observe(&DOWN, ConnectionCheck::Failing, 0, 0, None, start);
    assert_eq!(
        after(&mut tracker, start, 0, 0),
        Some(ConnectionNotice::WaitingForNetwork)
    );
    assert_eq!(
        after(&mut tracker, start, 300, 0),
        Some(ConnectionNotice::WaitingForNetwork)
    );
}

#[test]
fn a_returning_network_does_not_blame_the_server_for_the_time_without_it() {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    tracker.network_changed(DeviceNetwork::None, start);
    tracker.observe(&DOWN, ConnectionCheck::Ok, 0, 0, None, start);
    let back = start + Duration::from_secs(300);
    tracker.network_changed(DeviceNetwork::Wifi, back);
    assert_eq!(tracker.notice(&DOWN, back), None);
    assert_eq!(
        after(&mut tracker, back, 3, 0),
        Some(ConnectionNotice::Connecting)
    );

    // 没网期间一直在失败，也不把那些失败算到恢复后的这一轮：恢复后再失败一次才重新说连不上。
    let mut tracker = Tracker::default();
    tracker.network_changed(DeviceNetwork::None, start);
    assert_eq!(
        after(&mut tracker, start, 60, 1),
        Some(ConnectionNotice::WaitingForNetwork)
    );
    assert_eq!(
        after(&mut tracker, start, 120, 2),
        Some(ConnectionNotice::WaitingForNetwork)
    );
    tracker.network_changed(DeviceNetwork::Wifi, back);
    tracker.observe(&DOWN, ConnectionCheck::Ok, 2, 0, None, back);
    assert_eq!(tracker.notice(&DOWN, back), None);
    assert_eq!(
        after(&mut tracker, back, 3, 2),
        Some(ConnectionNotice::Connecting)
    );
    assert_eq!(
        after(&mut tracker, back, 4, 3),
        Some(ConnectionNotice::ServerOffline { retrying: false })
    );
}

#[test]
fn a_live_connection_wins_over_what_the_system_says() {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    tracker.network_changed(DeviceNetwork::None, start);
    tracker.observe(
        &HomeStatus::Connected,
        ConnectionCheck::Ok,
        0,
        0,
        None,
        start,
    );
    assert_eq!(tracker.notice(&HomeStatus::Connected, start), None);
}

/// 断开又连上：主人看得见的断开（期间提示过一句）恢复后开 4 秒的窗口，连着时的心跳重算不延长
/// 也不提前关掉它；到点翻回去，再断开马上收掉。
#[test]
fn the_recovery_feedback_lasts_four_seconds_and_stops_at_the_next_outage() {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    let at = |seconds| start + Duration::from_secs(seconds);
    tracker.observe(&DOWN, ConnectionCheck::Ok, 0, 0, None, start);
    // 断开期间提示过一句（publish 每秒重算一次）：这一次恢复才算“看得见”。
    assert_eq!(
        tracker.notice(&DOWN, at(3)),
        Some(ConnectionNotice::Connecting)
    );

    // 连上：窗口开着，期间每秒一次的连着重算不动它。
    tracker.observe(
        &HomeStatus::Connected,
        ConnectionCheck::Ok,
        0,
        0,
        None,
        at(10),
    );
    assert!(tracker.just_recovered(&None, at(10)));
    tracker.observe(
        &HomeStatus::Connected,
        ConnectionCheck::Ok,
        0,
        0,
        None,
        at(11),
    );
    assert!(tracker.just_recovered(&None, at(11)));
    // 窗口里有“更新中”就不显示恢复：壳只按核心给的画。
    assert!(!tracker.just_recovered(&Some(ConnectionNotice::Updating), at(11)));
    // 到点要有一次重算把提示翻回去：剩下的窗口不到一秒时，tick 提前到窗口结束。
    assert_eq!(
        tracker.next_tick(true, at(11)),
        Some(Duration::from_secs(1))
    );
    assert_eq!(
        tracker.next_tick(true, at(13) + Duration::from_millis(500)),
        Some(Duration::from_millis(500))
    );

    // 到点翻回去；再断开（这次也提示过一句）马上收掉，连上又开一个新窗口。
    assert!(!tracker.just_recovered(&None, at(14)));
    assert!(!tracker.just_recovered(&None, at(15)));
    tracker.observe(&DOWN, ConnectionCheck::Ok, 0, 0, None, at(16));
    assert_eq!(
        tracker.next_tick(true, at(16)),
        Some(Duration::from_secs(1))
    );
    assert_eq!(
        tracker.notice(&DOWN, at(19)),
        Some(ConnectionNotice::Connecting)
    );
    tracker.observe(
        &HomeStatus::Connected,
        ConnectionCheck::Ok,
        0,
        0,
        None,
        at(20),
    );
    assert!(tracker.just_recovered(&None, at(20)));
}

/// 一直连着（没有断开过）就谈不上恢复：每秒的心跳重算不开窗口。宽限内（两秒）就回来的抖动
/// 没提示过一句，恢复也不提示。
#[test]
fn a_connection_that_never_dropped_visibly_shows_no_recovery() {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    tracker.observe(
        &HomeStatus::Connected,
        ConnectionCheck::Ok,
        0,
        0,
        None,
        start,
    );
    assert!(!tracker.just_recovered(&None, start));
    tracker.observe(
        &HomeStatus::Connected,
        ConnectionCheck::Ok,
        0,
        0,
        None,
        start + Duration::from_secs(1),
    );
    assert!(!tracker.just_recovered(&None, start + Duration::from_secs(1)));

    // 一秒的抖动：期间提示是空（宽限挡住），连上后不开窗口。
    let mut tracker = Tracker::default();
    tracker.observe(&DOWN, ConnectionCheck::Ok, 0, 0, None, start);
    assert_eq!(
        tracker.notice(&DOWN, start + Duration::from_millis(500)),
        None
    );
    tracker.observe(
        &HomeStatus::Connected,
        ConnectionCheck::Ok,
        0,
        0,
        None,
        start + Duration::from_secs(1),
    );
    assert!(!tracker.just_recovered(&None, start + Duration::from_secs(1)));
}

#[test]
fn a_rejected_device_is_told_at_once() {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    tracker.observe(&rejected(), ConnectionCheck::Ok, 0, 0, None, start);
    assert_eq!(
        tracker.notice(&rejected(), start),
        Some(ConnectionNotice::DeviceRejected)
    );
}

#[test]
fn retry_progress_ends_after_failure_or_at_the_diagnostic_deadline() {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    tracker.observe(&DOWN, ConnectionCheck::Ok, 1, 0, None, start);
    let clicked = start + Duration::from_secs(40);
    tracker.retry_started(clicked);
    let progress = Some(ConnectionNotice::ServerOffline { retrying: true });
    let failure = Some(ConnectionNotice::ServerOffline { retrying: false });
    assert_eq!(tracker.notice(&DOWN, clicked), progress);
    tracker.retry_finished();
    assert_eq!(
        tracker.notice(&DOWN, clicked + RETRY_SPIN - Duration::from_nanos(1)),
        progress
    );
    assert_eq!(tracker.notice(&DOWN, clicked + RETRY_SPIN), failure);

    tracker.retry_started(clicked);
    assert_eq!(
        tracker.notice(&DOWN, clicked + LONG_OUTAGE - Duration::from_nanos(1)),
        progress
    );
    assert_eq!(tracker.notice(&DOWN, clicked + LONG_OUTAGE), failure);
    tracker.retry_started(clicked + LONG_OUTAGE);
    assert_eq!(tracker.notice(&DOWN, clicked + LONG_OUTAGE), progress);
}

#[test]
fn a_new_dial_clears_the_previous_offline_conclusion_until_it_finishes_or_times_out() {
    let start = Instant::now();
    for relay in [ConnectionCheck::Ok, ConnectionCheck::Failing] {
        let mut tracker = Tracker::default();
        tracker.observe(&DOWN, relay, 1, 0, None, start);
        let retry = start + Duration::from_secs(40);
        let failure = tracker.notice(&DOWN, retry);
        assert!(matches!(
            failure,
            Some(
                ConnectionNotice::ServerOffline { retrying: false }
                    | ConnectionNotice::Unreachable { retrying: false }
            )
        ));

        tracker.observe(&HomeStatus::Connecting, relay, 1, 0, None, retry);
        assert_eq!(
            tracker.notice(&HomeStatus::Connecting, retry),
            Some(ConnectionNotice::Connecting)
        );
        let boundary = retry + LONG_OUTAGE;
        tracker.observe(
            &HomeStatus::Connecting,
            relay,
            1,
            0,
            None,
            boundary - Duration::from_nanos(1),
        );
        assert_eq!(
            tracker.notice(&HomeStatus::Connecting, boundary - Duration::from_nanos(1)),
            Some(ConnectionNotice::Connecting)
        );
        tracker.observe(&HomeStatus::Connecting, relay, 1, 0, None, boundary);
        assert_eq!(tracker.notice(&HomeStatus::Connecting, boundary), failure);

        tracker.observe(&DOWN, relay, 2, 0, None, boundary);
        assert_eq!(tracker.notice(&DOWN, boundary), failure);
        tracker.observe(&HomeStatus::Connecting, relay, 2, 0, None, boundary);
        assert_eq!(
            tracker.notice(&HomeStatus::Connecting, boundary),
            Some(ConnectionNotice::Connecting)
        );
        tracker.observe(&HomeStatus::Connected, relay, 0, 0, None, boundary);
        assert_eq!(tracker.notice(&HomeStatus::Connected, boundary), None);
    }
}

#[test]
fn a_server_that_connects_but_never_answers_is_named_after_the_diagnostic_window() {
    // 建连每次都成功（失败次数一直是 0），探活 8 秒没有回应就断开、马上重拨：一轮约 9 秒。
    let start = Instant::now();
    for (relay, named) in [
        (
            ConnectionCheck::Ok,
            ConnectionNotice::ServerOffline { retrying: false },
        ),
        (
            ConnectionCheck::Failing,
            ConnectionNotice::Unreachable { retrying: false },
        ),
    ] {
        let mut tracker = Tracker::default();
        let mut at = |status: &HomeStatus, millis: u64| {
            let now = start + Duration::from_millis(millis);
            tracker.observe(status, relay, 0, 0, None, now);
            tracker.notice(status, now)
        };
        at(&HomeStatus::Connecting, 0);
        let mut seen = Vec::new();
        for round in 0..8 {
            let dialed = round * 9_000;
            seen.push((dialed + 3_000, at(&HomeStatus::Connecting, dialed + 3_000)));
            seen.push((dialed + 8_000, at(&DOWN, dialed + 8_000)));
        }
        for (millis, notice) in seen {
            let expected = if millis < 30_000 {
                ConnectionNotice::Connecting
            } else {
                named
            };
            assert_eq!(notice, Some(expected), "{millis} ms");
        }

        // 服务器终于回应了：恢复如常。
        assert_eq!(at(&HomeStatus::Connected, 80_000), None);
    }
}

#[test]
fn catching_up_shows_only_while_connected() {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    tracker.set_catching_up(true);
    tracker.observe(
        &HomeStatus::Connected,
        ConnectionCheck::Ok,
        0,
        0,
        None,
        start,
    );
    assert_eq!(
        tracker.notice(&HomeStatus::Connected, start),
        Some(ConnectionNotice::Updating)
    );
    tracker.observe(&DOWN, ConnectionCheck::Ok, 0, 0, None, start);
    assert_eq!(
        after(&mut tracker, start, 3, 0),
        Some(ConnectionNotice::Connecting)
    );
}

/// 断开后自动重试回到“连接中”，连接仍算已知不通；连上、回前台后不再算。
#[test]
fn a_failure_is_remembered_while_retrying_until_connected() {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    tracker.observe(
        &HomeStatus::Connecting,
        ConnectionCheck::Ok,
        0,
        0,
        None,
        start,
    );
    assert_eq!(tracker.known_failure(), None, "第一次建连还没失败过");
    tracker.observe(&DOWN, ConnectionCheck::Ok, 0, 0, None, start);
    tracker.observe(
        &HomeStatus::Connecting,
        ConnectionCheck::Ok,
        1,
        0,
        None,
        start,
    );
    assert_eq!(
        tracker.known_failure(),
        Some(HomeFailure::TransportUnavailable)
    );
    tracker.observe(
        &HomeStatus::Connected,
        ConnectionCheck::Ok,
        0,
        0,
        None,
        start,
    );
    assert_eq!(tracker.known_failure(), None);
    tracker.observe(&DOWN, ConnectionCheck::Ok, 0, 0, None, start);
    tracker.foregrounded(start);
    assert_eq!(tracker.known_failure(), None, "回前台先给新一轮建连机会");
}

/// 断开和自动重拨挤在两次发布之间：“断开”这一态被 watch 合并掉，只观察到“连接中”。
/// 统计是现场读的，断开在 `disconnects` 的增量里留了痕，闸门的依据照样要武装上，不能时拦时不拦。
#[test]
fn a_disconnect_merged_away_by_the_status_push_still_arms_the_gate() {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    tracker.observe(
        &HomeStatus::Connected,
        ConnectionCheck::Ok,
        0,
        0,
        None,
        start,
    );
    // 下一次发布只看到“连接中”，但统计里多了一次断开、带着原因。
    tracker.observe(
        &HomeStatus::Connecting,
        ConnectionCheck::Ok,
        0,
        1,
        Some(HomeFailure::TransportUnavailable),
        start,
    );
    assert_eq!(
        tracker.known_failure(),
        Some(HomeFailure::TransportUnavailable)
    );
    // 之后的“连接中”没有新的断开：武装保持，不被冲掉。
    tracker.observe(
        &HomeStatus::Connecting,
        ConnectionCheck::Ok,
        0,
        1,
        None,
        start,
    );
    assert_eq!(
        tracker.known_failure(),
        Some(HomeFailure::TransportUnavailable)
    );
    // 连上才算恢复。
    tracker.observe(
        &HomeStatus::Connected,
        ConnectionCheck::Ok,
        0,
        1,
        None,
        start,
    );
    assert_eq!(tracker.known_failure(), None);
}

#[test]
fn coming_back_restarts_the_clock_and_recovery_reports_the_outage() {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    tracker.observe(&DOWN, ConnectionCheck::Ok, 0, 0, None, start);
    let back = start + Duration::from_secs(600);
    tracker.foregrounded(back);
    assert_eq!(tracker.notice(&DOWN, back), None);
    // 回前台后的新失败如实显示，不能沿用旧的重试进度。
    assert_eq!(
        after(&mut tracker, back, 4, 1),
        Some(ConnectionNotice::ServerOffline { retrying: false })
    );
    let recovered = back + Duration::from_secs(5);
    assert_eq!(
        tracker.observe(
            &HomeStatus::Connected,
            ConnectionCheck::Ok,
            0,
            0,
            None,
            recovered
        ),
        Some(Recovery {
            outage: Duration::from_secs(5),
            trigger: "foreground",
        })
    );
}
