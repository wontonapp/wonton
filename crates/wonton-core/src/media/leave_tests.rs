use std::sync::Arc;

use axum::http::StatusCode;
use axum::routing::post;
use serde_json::json;
use wonton_protocol::media::{
    MediaKind, MediaOperation, MediaTask, PlaybackSession, PlaybackSessionId, StopReason,
    TaskState, new_task_id,
};

use super::Media;
use crate::http::HttpClient;
use crate::media::provider::credential_name;
use crate::media::test_support::{FakeDirectory, candidate, caps, serve};

const PROVIDER: &str = "app.example.media";

fn task(person: &str, state: &str) -> MediaTask {
    serde_json::from_value(json!({
        "id": new_task_id(), "kind": "download", "person": person, "device": "phone",
        "target": {"kind": "download", "value": {
            "id": "v1", "kind": "video", "title": "海边", "artist": null, "album": null,
            "artwork_url": null, "duration_ms": null, "series": null, "index": null,
            "played": false, "starred": false,
        }},
        "state": state, "progress": {"done": 0, "total": null, "unit": "bytes"},
        "retry_count": 0, "retry_of": null, "next_attempt_at": null, "created_at": 0,
        "partial_failures": [],
    }))
    .unwrap()
}

fn session(person: &str) -> PlaybackSession {
    serde_json::from_value(json!({
        "id": PlaybackSessionId::new_v4(), "person": person, "device": "phone",
        "item": {"id": "v1", "kind": "video", "title": "海边", "artist": null, "album": null,
                 "artwork_url": null, "duration_ms": 10000, "series": null, "index": null,
                 "played": false, "starred": false},
        "state": "running", "position_ms": 4000, "plan": null, "stop_reason": null,
    }))
    .unwrap()
}

#[tokio::test]
async fn leaving_cancels_tasks_stops_sessions_and_revokes_only_that_person() {
    let router =
        caps(&["playback"], &["video"]).route("/revoke", post(|| async { StatusCode::NO_CONTENT }));
    let (endpoint, seen, _served) = serve(router).await;
    let entry = candidate(PROVIDER, &[MediaOperation::Playback], &[MediaKind::Video]);
    let (directory, book) = FakeDirectory::new(vec![(entry, endpoint)]);
    let dir = tempfile::tempdir().unwrap();
    let media = Media::with_directory(dir.path(), Arc::new(directory), HttpClient::new()).unwrap();
    let anyone = |_: &str| true;
    let (alice, bob) = ("@alice:example.com", "@bob:example.com");
    let (waiting, done, others) = (
        task(alice, "queued"),
        task(alice, "done"),
        task(bob, "queued"),
    );
    for task in [&waiting, &done, &others] {
        media.store.create_task(task).unwrap();
    }
    let playing = session(alice);
    media.store.create_session(&playing).unwrap();
    for person in [alice, bob] {
        media
            .providers
            .route_to(person, anyone, PROVIDER)
            .await
            .unwrap();
    }

    media.leave(alice).await.unwrap();
    let state = |task: &MediaTask| media.store.task_any(task.id).unwrap().state;
    assert_eq!(
        [state(&waiting), state(&done), state(&others)],
        [TaskState::Cancelled, TaskState::Done, TaskState::Queued]
    );
    let stopped = media.store.stored_session(playing.id).unwrap().session;
    assert_eq!(
        (stopped.stop_reason, stopped.position_ms),
        (Some(StopReason::Removed), 4000)
    );
    assert_eq!(
        *book.revoked.lock().unwrap(),
        [(PROVIDER.to_owned(), credential_name(alice))]
    );
    // 再走一遍（成员移除重试）不再碰已收回的。
    media.leave(alice).await.unwrap();
    let revokes = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|(path, _, _)| path == "POST /revoke")
        .count();
    assert_eq!(revokes, 1);
}

#[tokio::test]
async fn an_uninstalled_provider_only_loses_the_cores_copy() {
    let (endpoint, seen, _served) = serve(caps(&["playback"], &["video"])).await;
    let entry = candidate(PROVIDER, &[MediaOperation::Playback], &[MediaKind::Video]);
    let dir = tempfile::tempdir().unwrap();
    let alice = "@alice:example.com";
    let (directory, _) = FakeDirectory::new(vec![(entry.clone(), endpoint.clone())]);
    let media = Media::with_directory(dir.path(), Arc::new(directory), HttpClient::new()).unwrap();
    media
        .providers
        .route_to(alice, |_| true, PROVIDER)
        .await
        .unwrap();
    drop(media);

    let (uninstalled, book) = FakeDirectory::new(Vec::new());
    let media =
        Media::with_directory(dir.path(), Arc::new(uninstalled), HttpClient::new()).unwrap();
    media.leave(alice).await.unwrap();
    assert!(book.revoked.lock().unwrap().is_empty());
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(path, _, _)| path == "POST /revoke")
    );
    drop(media);

    // 重装回来是一把新的：核心那把已经删了。
    let (directory, book) = FakeDirectory::new(vec![(entry, endpoint)]);
    let media = Media::with_directory(dir.path(), Arc::new(directory), HttpClient::new()).unwrap();
    media
        .providers
        .route_to(alice, |_| true, PROVIDER)
        .await
        .unwrap();
    assert_eq!(book.issued.lock().unwrap().len(), 1);
}
