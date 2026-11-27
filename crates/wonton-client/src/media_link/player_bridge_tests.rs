//! 播放桥的行为测试：给一串播放器事件，看发给核心的是什么、壳要做什么。

use std::time::Duration;

use tokio::time::Instant;
use wonton_protocol::media::provider::{PlaybackProgress, PlaybackStopped};
use wonton_protocol::media::{
    ControlOp, MediaFailure, MediaFailureCode, MediaItemRef, MediaKind, PlaybackSession,
    PlaybackSessionId, Queue, QueueMode, StopReason, TaskState,
};

use super::player_bridge::{Bridge, PlayerCommand, PlayerEvent, Report};

fn song(id: &str) -> MediaItemRef {
    MediaItemRef {
        id: id.into(),
        kind: MediaKind::Audio,
        title: id.into(),
        artist: None,
        album: None,
        artwork_url: None,
        duration_ms: Some(200_000),
        series: None,
        index: None,
        played: false,
        starred: false,
        source: None,
    }
}

fn session(stop: Option<StopReason>) -> PlaybackSession {
    PlaybackSession {
        id: PlaybackSessionId::new_v4(),
        person: "@alice:example.com".into(),
        device: "phone".into(),
        item: song("s1"),
        state: if stop.is_some() {
            TaskState::Cancelled
        } else {
            TaskState::Running
        },
        position_ms: 0,
        plan: None,
        stop_reason: stop,
    }
}

fn queue(len: usize, cursor: usize, mode: QueueMode) -> Queue {
    Queue {
        revision: 1,
        entries: (0..len).map(|i| song(&format!("s{i}"))).collect(),
        cursor: Some(cursor),
        mode,
        sleep_at_ms: None,
    }
}

fn playing(position_ms: u64) -> PlayerEvent {
    PlayerEvent::Playing { position_ms }
}

fn progress(position_ms: u64) -> Option<Report> {
    Some(Report::Progress(PlaybackProgress {
        position_ms,
        wall_clock: 7,
    }))
}

fn stop(position_ms: u64, reason: StopReason) -> Option<Report> {
    Some(Report::Stop(PlaybackStopped {
        position_ms,
        reason,
    }))
}

/// 在 `start` 之后 `ms` 毫秒报一个事件，回发给核心的。
fn report_at(
    bridge: &mut Bridge,
    session: &PlaybackSession,
    queue: Option<&Queue>,
    start: Instant,
    ms: u64,
    event: &PlayerEvent,
) -> Option<Report> {
    let now = start + Duration::from_millis(ms);
    bridge.decide(session, queue, event, now, 7).report
}

#[test]
fn while_playing_the_position_goes_out_at_most_every_five_seconds() {
    let mut bridge = Bridge::default();
    let session = session(None);
    let start = Instant::now();
    let sent: Vec<_> = [0, 1_000, 4_900, 5_000, 9_000, 10_000]
        .into_iter()
        .filter_map(|ms| report_at(&mut bridge, &session, None, start, ms, &playing(ms)))
        .collect();
    assert_eq!(
        sent,
        vec![
            progress(0).unwrap(),
            progress(5_000).unwrap(),
            progress(10_000).unwrap()
        ]
    );
}

#[test]
fn pausing_and_seeking_go_out_at_once_and_restart_the_rhythm() {
    let mut bridge = Bridge::default();
    let session = session(None);
    let start = Instant::now();
    assert_eq!(
        report_at(&mut bridge, &session, None, start, 0, &playing(0)),
        progress(0)
    );
    let paused = PlayerEvent::Paused { position_ms: 1_200 };
    assert_eq!(
        report_at(&mut bridge, &session, None, start, 1_200, &paused),
        progress(1_200)
    );
    let seeked = PlayerEvent::Seeked {
        position_ms: 90_000,
    };
    assert_eq!(
        report_at(&mut bridge, &session, None, start, 3_000, &seeked),
        progress(90_000)
    );
    // 拖动之后的节奏从拖动那一刻算：5 秒内的出声不报。
    assert_eq!(
        report_at(&mut bridge, &session, None, start, 7_000, &playing(94_000)),
        None
    );
    assert_eq!(
        report_at(&mut bridge, &session, None, start, 8_000, &playing(95_000)),
        progress(95_000)
    );
}

#[test]
fn a_new_session_is_reported_right_away() {
    let mut bridge = Bridge::default();
    let start = Instant::now();
    let first = session(None);
    assert_eq!(
        report_at(&mut bridge, &first, None, start, 0, &playing(0)),
        progress(0)
    );
    let second = session(None);
    assert_eq!(
        report_at(&mut bridge, &second, None, start, 1_000, &playing(0)),
        progress(0)
    );
}

#[test]
fn a_finished_song_moves_on_while_the_queue_has_more() {
    let mut bridge = Bridge::default();
    let session = session(None);
    let start = Instant::now();
    let done = PlayerEvent::Finished {
        position_ms: 200_000,
    };
    let next = Some(Report::Control(ControlOp::Next));

    let middle = queue(3, 1, QueueMode::Sequential);
    assert_eq!(
        report_at(&mut bridge, &session, Some(&middle), start, 0, &done),
        next
    );

    let last = queue(3, 2, QueueMode::Sequential);
    assert_eq!(
        report_at(&mut bridge, &session, Some(&last), start, 0, &done),
        stop(200_000, StopReason::Finished)
    );

    let looping = queue(3, 2, QueueMode::RepeatAll);
    assert_eq!(
        report_at(&mut bridge, &session, Some(&looping), start, 0, &done),
        next
    );

    // 单放一首（没有队列）放完就是整个放完。
    assert_eq!(
        report_at(&mut bridge, &session, None, start, 0, &done),
        stop(200_000, StopReason::Finished)
    );
}

#[test]
fn repeat_one_starts_the_song_over() {
    let mut bridge = Bridge::default();
    let session = session(None);
    let one = queue(3, 1, QueueMode::RepeatOne);
    let done = PlayerEvent::Finished {
        position_ms: 200_000,
    };
    let decision = bridge.decide(&session, Some(&one), &done, Instant::now(), 7);
    assert_eq!(decision.report, Some(Report::Control(ControlOp::SeekMs(0))));
    assert_eq!(decision.command, PlayerCommand::SeekTo { position_ms: 0 });
}

#[test]
fn stopping_here_and_failing_to_play_stop_the_session() {
    let mut bridge = Bridge::default();
    let session = session(None);
    let start = Instant::now();
    let stopped = PlayerEvent::Stopped {
        position_ms: 42_000,
    };
    assert_eq!(
        report_at(&mut bridge, &session, None, start, 0, &stopped),
        stop(42_000, StopReason::User)
    );
    let failed = PlayerEvent::Failed {
        position_ms: 1_000,
        code: MediaFailureCode::UnsupportedByDevice,
        message: "decoder".into(),
    };
    assert_eq!(
        report_at(&mut bridge, &session, None, start, 0, &failed),
        stop(1_000, StopReason::Error)
    );
}

#[test]
fn a_taken_over_session_gets_the_last_position_once_and_nothing_else() {
    let mut bridge = Bridge::default();
    let session = session(Some(StopReason::TakenOver));
    let start = Instant::now();
    // 壳还没停下来时的出声不报：会话已经不归这台设备了。
    assert_eq!(
        report_at(&mut bridge, &session, None, start, 0, &playing(61_000)),
        None
    );
    let stopped = PlayerEvent::Stopped {
        position_ms: 61_500,
    };
    assert_eq!(
        report_at(&mut bridge, &session, None, start, 500, &stopped),
        stop(61_500, StopReason::TakenOver)
    );
    assert_eq!(
        report_at(&mut bridge, &session, None, start, 900, &stopped),
        None
    );
}

#[test]
fn an_empty_queue_or_one_not_playing_from_it_ends_the_session() {
    let mut bridge = Bridge::default();
    let session = session(None);
    let start = Instant::now();
    let done = PlayerEvent::Finished {
        position_ms: 200_000,
    };
    let empty = Queue {
        entries: Vec::new(),
        cursor: None,
        ..queue(0, 0, QueueMode::RepeatAll)
    };
    assert_eq!(
        report_at(&mut bridge, &session, Some(&empty), start, 0, &done),
        stop(200_000, StopReason::Finished)
    );
    // 队列里有歌，但这首不是从队列里放的：没有“下一项”可言。
    let elsewhere = Queue {
        cursor: None,
        ..queue(3, 0, QueueMode::Sequential)
    };
    assert_eq!(
        report_at(&mut bridge, &session, Some(&elsewhere), start, 0, &done),
        stop(200_000, StopReason::Finished)
    );
}

#[test]
fn a_taken_over_session_gets_one_last_position_whatever_the_player_reports_first() {
    let start = Instant::now();
    let failed = PlayerEvent::Failed {
        position_ms: 30_000,
        code: MediaFailureCode::TransportUnavailable,
        message: "loopback closed".into(),
    };
    let seeked = PlayerEvent::Seeked {
        position_ms: 45_000,
    };
    let paused = PlayerEvent::Paused {
        position_ms: 45_000,
    };

    let mut bridge = Bridge::default();
    let taken = session(Some(StopReason::TakenOver));
    assert_eq!(
        report_at(&mut bridge, &taken, None, start, 0, &seeked),
        stop(45_000, StopReason::TakenOver)
    );
    for later in [&failed, &paused, &seeked] {
        assert_eq!(
            report_at(&mut bridge, &taken, None, start, 100, later),
            None
        );
    }

    let mut bridge = Bridge::default();
    let again = session(Some(StopReason::TakenOver));
    assert_eq!(
        report_at(&mut bridge, &again, None, start, 0, &failed),
        stop(30_000, StopReason::TakenOver)
    );
    assert_eq!(
        report_at(&mut bridge, &again, None, start, 100, &seeked),
        None
    );
}

#[test]
fn a_finished_session_without_a_stop_reason_still_keeps_the_last_position() {
    let start = Instant::now();
    let stopped = PlayerEvent::Stopped {
        position_ms: 30_000,
    };
    let cases = [
        (TaskState::Done, StopReason::Finished),
        (
            TaskState::Failed(MediaFailure {
                code: MediaFailureCode::UpstreamError,
                message: "provider gone".into(),
                retry_after_secs: None,
            }),
            StopReason::Error,
        ),
        (TaskState::Cancelled, StopReason::User),
    ];
    for (state, reason) in cases {
        let mut bridge = Bridge::default();
        let session = PlaybackSession {
            state,
            ..session(None)
        };
        assert_eq!(
            report_at(&mut bridge, &session, None, start, 0, &playing(29_000)),
            None
        );
        assert_eq!(
            report_at(&mut bridge, &session, None, start, 500, &stopped),
            stop(30_000, reason)
        );
        assert_eq!(
            report_at(&mut bridge, &session, None, start, 900, &stopped),
            None
        );
    }
}
