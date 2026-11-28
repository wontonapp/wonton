//! 客户端核媒体的行为测试：本机这份以服务器为准对齐、视图怎么翻、订阅退了就不再收、请求带设备头、
//! 失败怎么归类。

use reqwest::Method;
use tokio::sync::watch;
use url::Url;
use wonton_protocol::media::{
    DeviceProfile, ItemState, MediaFailure, MediaFailureCode, MediaItemRef, MediaKind, MediaTask,
    MediaViewModel, PlaybackRequest, PlaybackSession, Progress, ProgressUnit, StopReason, TaskKind,
    TaskState, TaskTarget, UploadItem, WONTON_DEVICE_HEADER, new_task_id,
};

use super::api::{MediaApi, classify};
use super::profile::{PlayerSupport, Profiles};
use super::state::Local;
use super::subscription::{MediaSubscription, PlaybackSubscription};
use super::{PlayerPhase, player_state};

const PERSON: &str = "@alice:example.com";

fn upload(offset: u64) -> MediaTask {
    MediaTask {
        id: new_task_id(),
        kind: TaskKind::Upload,
        person: PERSON.into(),
        device: "phone".into(),
        target: TaskTarget::Upload(vec![UploadItem {
            item_id: "i1".into(),
            path: "DCIM/Camera/1.jpg".into(),
            size: 1000,
            checksum: None,
            taken_at: None,
            state: ItemState::Running,
            upload_offset: offset,
        }]),
        state: TaskState::Running,
        progress: Progress {
            done: offset,
            total: Some(1000),
            unit: ProgressUnit::Bytes,
        },
        retry_count: 0,
        retry_of: None,
        next_attempt_at: None,
        created_at: 1,
        partial_failures: Vec::new(),
    }
}

fn session(stop: Option<StopReason>) -> PlaybackSession {
    PlaybackSession {
        id: wonton_protocol::media::PlaybackSessionId::new_v4(),
        person: PERSON.into(),
        device: "phone".into(),
        item: MediaItemRef {
            id: "v1".into(),
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
        state: if stop.is_some() {
            TaskState::Cancelled
        } else {
            TaskState::Running
        },
        position_ms: 4_000,
        plan: None,
        stop_reason: stop,
    }
}

fn offset_in(view: &MediaViewModel) -> u64 {
    view.tasks[0].items[0].upload_offset
}

#[test]
fn reconnecting_replaces_local_offsets_with_the_servers() {
    let mut local = Local::default();
    let task = upload(300);
    local.align(vec![task.clone()], Vec::new());
    local.advance_upload(task.id, "i1", 900);
    assert_eq!(offset_in(&local.media_view()), 900);
    assert_eq!(local.media_view().tasks[0].progress.done, 900);

    // 断线期间服务器只收到了 400：连上后以它为准，本地推过头的部分作废。
    let mut server = task.clone();
    if let TaskTarget::Upload(items) = &mut server.target {
        items[0].upload_offset = 400;
    }
    server.progress.done = 400;
    local.align(vec![server], Vec::new());
    assert_eq!(offset_in(&local.media_view()), 400);
    assert_eq!(local.media_view().tasks[0].progress.done, 400);
}

#[test]
fn tasks_the_server_no_longer_has_disappear() {
    let mut local = Local::default();
    local.put_task(upload(0));
    local.align(Vec::new(), Vec::new());
    assert!(local.media_view().tasks.is_empty());
}

#[test]
fn a_failed_pull_is_shown_until_the_next_one_lands() {
    let mut local = Local::default();
    local.tasks_failed(MediaFailure {
        code: MediaFailureCode::TransportUnavailable,
        message: "断了".into(),
        retry_after_secs: None,
    });
    assert_eq!(
        local.media_view().failure.map(|f| f.code),
        Some(MediaFailureCode::TransportUnavailable)
    );
    local.align(Vec::new(), Vec::new());
    assert!(local.media_view().failure.is_none());
}

#[test]
fn a_failed_task_shows_its_reason_and_items_their_file_names() {
    let mut task = upload(0);
    let failure = MediaFailure {
        code: MediaFailureCode::AuthenticationRequired,
        message: "401".into(),
        retry_after_secs: None,
    };
    task.state = TaskState::Failed(failure.clone());
    let mut local = Local::default();
    local.align(vec![task], Vec::new());
    let view = local.media_view();
    assert_eq!(view.tasks[0].failure, Some(failure));
    assert_eq!(view.tasks[0].items[0].name, "1.jpg");
    assert_eq!(view.tasks[0].title, "1.jpg");
}

#[test]
fn the_session_the_server_still_has_active_wins_and_a_stopped_one_stays_visible() {
    let mut local = Local::default();
    let active = session(None);
    local.align(
        Vec::new(),
        vec![session(Some(StopReason::User)), active.clone()],
    );
    assert_eq!(local.playback_view().unwrap().session.id, active.id);

    let mut stopped = active.clone();
    stopped.stop_reason = Some(StopReason::User);
    stopped.state = TaskState::Cancelled;
    local.put_session(stopped);
    local.align(Vec::new(), Vec::new());
    let view = local.playback_view().unwrap();
    assert_eq!(view.session.id, active.id);
    assert_eq!(
        player_state(Some(&view.session)).phase,
        PlayerPhase::Stopped
    );
}

#[test]
fn player_state_follows_the_session() {
    assert_eq!(player_state(None).phase, PlayerPhase::Stopped);
    let waiting = session(None);
    assert_eq!(player_state(Some(&waiting)).phase, PlayerPhase::Buffering);
    assert_eq!(player_state(Some(&waiting)).position_ms, 4_000);
    // 暂停中正在重取计划：还是暂停。
    let mut paused = session(None);
    paused.state = TaskState::Paused(wonton_protocol::media::PauseReason::User);
    assert_eq!(player_state(Some(&paused)).phase, PlayerPhase::Paused);
}

#[test]
fn a_playback_failure_stays_until_the_next_action_lands() {
    let mut local = Local::default();
    let current = session(None);
    local.put_session(current.clone());
    local.playback_failed(MediaFailure {
        code: MediaFailureCode::TransportUnavailable,
        message: "断了".into(),
        retry_after_secs: None,
    });
    // 同一条会话的回应不算“下一次成了”的信号，动作成了才清。
    local.put_session(current);
    assert!(local.playback_view().unwrap().failure.is_some());
    local.playback_succeeded();
    assert!(local.playback_view().unwrap().failure.is_none());
}

#[tokio::test]
async fn a_cancelled_subscription_gets_nothing_more() {
    let (sender, receiver) = watch::channel(Local::default().media_view());
    let subscription = MediaSubscription::new(receiver);
    assert!(subscription.next().await.is_some());
    subscription.cancel();
    let mut local = Local::default();
    local.put_task(upload(0));
    sender.send_replace(local.media_view());
    assert!(subscription.next().await.is_none());
}

#[tokio::test]
async fn playback_subscription_waits_for_a_session() {
    let (sender, receiver) = watch::channel(None);
    let subscription = PlaybackSubscription::new(receiver);
    let mut local = Local::default();
    local.put_session(session(None));
    let view = local.playback_view();
    let next = tokio::spawn(async move { subscription.next().await });
    tokio::task::yield_now().await;
    sender.send_replace(view.clone());
    assert_eq!(next.await.unwrap(), view);
}

#[test]
fn every_request_names_this_device() {
    let api = MediaApi::new(
        Url::parse("http://core.example.com").unwrap(),
        reqwest::Client::new(),
        "endpoint-id".into(),
    );
    for method in [Method::GET, Method::POST, Method::PATCH] {
        let request = api
            .request(method, "/_wonton/api/media/tasks")
            .build()
            .unwrap();
        assert_eq!(request.headers()[WONTON_DEVICE_HEADER], "endpoint-id");
    }
}

#[test]
fn failures_keep_the_cores_category_and_name_the_tunnel() {
    let envelope = br#"{"code":"rate_limited","message":"slow","retry_after_secs":30}"#;
    let kept = classify(429, None, envelope);
    assert_eq!(
        (kept.code, kept.retry_after_secs),
        (MediaFailureCode::RateLimited, Some(30))
    );
    assert_eq!(
        classify(502, Some("unreachable"), b"").code,
        MediaFailureCode::TransportUnavailable
    );
    // 没有信封的按 ADR-0113 第 9 节的表分，表外的 5xx 算那头出错，其余才算协议对不上。
    for (status, code) in [
        (400, MediaFailureCode::Protocol),
        (401, MediaFailureCode::AuthenticationRequired),
        (403, MediaFailureCode::PermissionDenied),
        (404, MediaFailureCode::MediaGone),
        (429, MediaFailureCode::RateLimited),
        (502, MediaFailureCode::UpstreamError),
        (503, MediaFailureCode::PluginUnavailable),
        (507, MediaFailureCode::ServerStorageFull),
        (500, MediaFailureCode::UpstreamError),
        (418, MediaFailureCode::Protocol),
    ] {
        assert_eq!(classify(status, None, b"<html>").code, code, "{status}");
    }
}

fn support(mime_types: &[&str]) -> PlayerSupport {
    PlayerSupport {
        mime_types: mime_types.iter().map(|s| (*s).to_owned()).collect(),
        containers: vec!["mp4".into()],
        max_bitrate: 0,
        hdr: true,
    }
}

#[test]
fn a_start_without_a_profile_uses_the_reported_player_even_after_a_restart() {
    let dir = tempfile::TempDir::new().unwrap();
    let profiles = Profiles::open(dir.path());
    let mut request = PlaybackRequest {
        position_ms: 0,
        audio: None,
        subtitle: None,
        item: session(None).item,
        device: String::new(),
        person: String::new(),
        profile: DeviceProfile {
            direct_play: Vec::new(),
            max_bitrate: 0,
            hdr: false,
            container: Vec::new(),
        },
    };
    let unsupported = Some(MediaFailureCode::UnsupportedByDevice);
    assert_eq!(
        profiles.fill(&mut request).err().map(|f| f.code),
        unsupported
    );
    // 报了空的不记：空数组到提供方那边会把能直放的也转码。
    assert_eq!(
        profiles.report(support(&[" "])).err().map(|f| f.code),
        unsupported
    );

    profiles
        .report(support(&["video/MP4", "video/mp4"]))
        .unwrap();
    Profiles::open(dir.path()).fill(&mut request).unwrap();
    assert_eq!(request.profile.direct_play, vec!["video/mp4".to_owned()]);
    assert_eq!(request.profile.max_bitrate, u32::MAX);
}
