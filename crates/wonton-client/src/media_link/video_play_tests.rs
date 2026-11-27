//! 起播回来的会话怎么翻成播放器的地址，位置多久报一次。

use wonton_protocol::media::{
    MediaFailureCode, MediaItemRef, MediaKind, PlaybackPlan, PlaybackSession, PlaybackSessionId,
    PlaybackViewModel, ProviderStream, TaskState,
};

use super::{Reports, playback};

const PREFIX: &str = "http://127.0.0.1:4100/t0ken";

fn view(stream: ProviderStream) -> PlaybackViewModel {
    let session = PlaybackSession {
        id: PlaybackSessionId::new_v4(),
        person: "@alice:example.com".into(),
        device: "phone".into(),
        item: MediaItemRef {
            id: "m1".into(),
            kind: MediaKind::Video,
            title: "海边".into(),
            artist: None,
            album: None,
            artwork_url: None,
            duration_ms: Some(10_000),
            series: None,
            index: None,
            played: false,
            starred: false,
            source: None,
        },
        state: TaskState::Running,
        position_ms: 4_000,
        plan: None,
        stop_reason: None,
    };
    let plan = PlaybackPlan {
        session: "up-1".into(),
        stream,
        origin_stream: None,
        segments: Vec::new(),
        audio: Vec::new(),
        subtitles: Vec::new(),
        seekable: true,
        duration_ms: 10_000,
        expires_at: 0,
    };
    PlaybackViewModel {
        session,
        queue: None,
        plan: Some(plan),
        failure: None,
    }
}

#[test]
fn the_player_opens_the_cores_stream_through_the_loopback() {
    let view = view(ProviderStream::Transcoded(
        "/_wonton/api/media/sessions/s1/stream/master.m3u8?DeviceId=d".into(),
    ));
    let playback = playback(PREFIX, &view).unwrap();
    assert_eq!(
        playback.url,
        "http://127.0.0.1:4100/t0ken/sessions/s1/stream/master.m3u8?DeviceId=d"
    );
    assert!(playback.transcoded);
    assert_eq!(playback.position_ms, 4_000);
}

#[test]
fn a_stream_outside_the_media_api_is_not_opened() {
    let view = view(ProviderStream::Direct("http://jellyfin:8096/x".into()));
    assert_eq!(
        playback(PREFIX, &view).unwrap_err().code,
        MediaFailureCode::Protocol
    );
}

#[test]
fn positions_are_reported_on_a_pace_unless_forced() {
    let reports = Reports::default();
    let (one, two) = (PlaybackSessionId::new_v4(), PlaybackSessionId::new_v4());
    assert!(reports.due(one, false));
    assert!(!reports.due(one, false));
    assert!(reports.due(one, true));
    assert!(reports.due(two, false));
}
