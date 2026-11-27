//! 选提供方、按人签凭据、失败归类的行为测试：假提供方是本机的 HTTP 服务，假目录记下签发和收回。

use std::sync::Arc;

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::post;
use serde_json::json;
use tokio::net::TcpListener;
use wonton_protocol::media::uploads::UploadStart;
use wonton_protocol::media::{
    MediaFailure, MediaFailureCode, MediaKind, MediaOperation, MediaTask, Progress, ProgressUnit,
    TaskKind, TaskState, TaskTarget, WONTON_USER_HEADER, new_task_id,
};

use super::provider::{Candidate, MediaProviders, credential_name};
use super::test_support::{Book, FakeDirectory, candidate, caps, serve};
use super::{Store, TaskService};
use crate::http::HttpClient;
use crate::plugin::upstreams::Endpoint;

fn providers(
    dir: &tempfile::TempDir,
    entries: Vec<(Candidate, Endpoint)>,
) -> (MediaProviders, Book) {
    let (directory, book) = FakeDirectory::new(entries);
    (
        MediaProviders::new(
            Arc::new(directory),
            HttpClient::new(),
            dir.path().join("keys"),
        ),
        book,
    )
}

fn photos() -> Candidate {
    candidate(
        "app.example.photos",
        &[MediaOperation::Upload],
        &[MediaKind::Photo, MediaKind::Video],
    )
}

fn upload() -> UploadStart {
    UploadStart {
        path: "DCIM/1.jpg".into(),
        size: 10,
        checksum: None,
        taken_at: None,
        live_video_checksum: None,
    }
}

fn anyone(_: &str) -> bool {
    true
}

fn running_upload(person: &str) -> MediaTask {
    MediaTask {
        id: new_task_id(),
        kind: TaskKind::Upload,
        person: person.into(),
        device: "phone".into(),
        target: TaskTarget::Upload(Vec::new()),
        state: TaskState::Running,
        progress: Progress {
            done: 0,
            total: Some(10),
            unit: ProgressUnit::Bytes,
        },
        retry_count: 0,
        retry_of: None,
        next_attempt_at: None,
        created_at: 0,
        partial_failures: Vec::new(),
    }
}

#[tokio::test]
async fn upstream_401_fails_the_task_without_backoff() {
    let router = caps(&["upload"], &["photo"])
        .route("/uploads", post(|| async { StatusCode::UNAUTHORIZED }));
    let (endpoint, _seen, _served) = serve(router).await;
    let dir = tempfile::tempdir().unwrap();
    let (providers, _) = providers(&dir, vec![(photos(), endpoint)]);
    let route = providers
        .route("alice", anyone, &MediaOperation::Upload, None, None)
        .await
        .unwrap();
    let failure = route
        .client
        .start_upload("alice", &upload())
        .await
        .unwrap_err();
    assert_eq!(failure.code, MediaFailureCode::AuthenticationRequired);

    let store = Arc::new(Store::open(dir.path().join("media")).unwrap());
    let task = running_upload("alice");
    store.create_task(&task).unwrap();
    TaskService::new(store.clone())
        .fail(task.id, failure)
        .unwrap();
    let task = store.task("alice", task.id).unwrap().unwrap();
    assert!(matches!(
        task.state,
        TaskState::Failed(MediaFailure {
            code: MediaFailureCode::AuthenticationRequired,
            ..
        })
    ));
    assert_eq!(task.retry_count, 0);
    assert_eq!(task.next_attempt_at, None);
}

#[tokio::test]
async fn person_outside_the_plugin_scope_gets_no_credential() {
    let (endpoint, seen, _served) = serve(caps(&["upload"], &["photo"])).await;
    let dir = tempfile::tempdir().unwrap();
    let (providers, book) = providers(&dir, vec![(photos(), endpoint)]);
    let failure = providers
        .route("bob", |_| false, &MediaOperation::Upload, None, None)
        .await
        .unwrap_err();
    assert_eq!(failure.code, MediaFailureCode::PermissionDenied);
    assert!(book.issued.lock().unwrap().is_empty());
    assert!(seen.lock().unwrap().is_empty());
    let failure = providers
        .route_to(
            "bob",
            |_| false,
            "app.example.photos",
            &MediaOperation::Upload,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(failure.code, MediaFailureCode::PermissionDenied);
}

#[tokio::test]
async fn each_person_gets_one_key_and_every_request_carries_it() {
    let (endpoint, seen, _served) = serve(caps(&["upload"], &["photo"])).await;
    let dir = tempfile::tempdir().unwrap();
    let (providers, book) = providers(&dir, vec![(photos(), endpoint)]);
    for person in ["alice", "alice", "bob"] {
        providers
            .route(person, anyone, &MediaOperation::Upload, None, None)
            .await
            .unwrap()
            .client
            .capabilities(person)
            .await
            .unwrap();
    }
    let issued = book.issued.lock().unwrap().clone();
    assert_eq!(
        issued
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect::<Vec<_>>(),
        [credential_name("alice"), credential_name("bob")]
    );
    let key_of = |person: &str| {
        issued
            .iter()
            .find(|(_, name, _)| *name == credential_name(person))
            .map(|(_, _, key)| format!("Bearer {key}"))
    };
    assert_ne!(key_of("alice"), key_of("bob"));
    for (path, auth, user) in seen.lock().unwrap().iter() {
        assert_eq!(path, "GET /_wonton/capabilities");
        assert_eq!(*auth, key_of(user.as_deref().unwrap()));
    }
}

#[tokio::test]
async fn picks_the_provider_by_operation_and_kind() {
    let (photos_at, _, _p) = serve(caps(&["upload"], &["photo", "video"])).await;
    let (video_at, _, _v) = serve(caps(&["playback", "download", "list"], &["video"])).await;
    let dir = tempfile::tempdir().unwrap();
    let video = candidate(
        "app.example.video",
        &[
            MediaOperation::Playback,
            MediaOperation::Download,
            MediaOperation::List,
        ],
        &[MediaKind::Video],
    );
    let (providers, _) = providers(&dir, vec![(photos(), photos_at), (video, video_at)]);
    let pick = async |operation: MediaOperation, kind: Option<MediaKind>| {
        providers
            .route("alice", anyone, &operation, kind.as_ref(), None)
            .await
            .map(|route| route.provider)
    };
    assert_eq!(
        pick(MediaOperation::Upload, None).await.unwrap(),
        "app.example.photos"
    );
    assert_eq!(
        pick(MediaOperation::Playback, Some(MediaKind::Video))
            .await
            .unwrap(),
        "app.example.video"
    );
    assert_eq!(
        pick(MediaOperation::Playback, Some(MediaKind::Audio))
            .await
            .unwrap_err()
            .code,
        MediaFailureCode::PluginUnavailable
    );
}

#[tokio::test]
async fn runtime_capabilities_win_over_the_manifest() {
    // 清单说能放音乐，运行时只报视频：不选它。
    let (endpoint, _, _served) = serve(caps(&["playback"], &["video"])).await;
    let dir = tempfile::tempdir().unwrap();
    let declared = candidate(
        "app.example.music",
        &[MediaOperation::Playback],
        &[MediaKind::Audio, MediaKind::Video],
    );
    let (providers, _) = providers(&dir, vec![(declared, endpoint)]);
    let failure = providers
        .route(
            "alice",
            anyone,
            &MediaOperation::Playback,
            Some(&MediaKind::Audio),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(failure.code, MediaFailureCode::PluginUnavailable);
    // 记下的那家同样核对：会话建好后提供方不再报音乐，续播不照发过去。
    let failure = providers
        .route_to(
            "alice",
            anyone,
            "app.example.music",
            &MediaOperation::Playback,
            Some(&MediaKind::Audio),
        )
        .await
        .unwrap_err();
    assert_eq!(failure.code, MediaFailureCode::PluginUnavailable);
    // 清单就没写的操作也一样，不等第一个请求被提供方拒。
    let failure = providers
        .route_to(
            "alice",
            anyone,
            "app.example.music",
            &MediaOperation::Upload,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(failure.code, MediaFailureCode::PluginUnavailable);
}

#[tokio::test]
async fn upload_check_asks_in_batches_and_merges_what_is_already_there() {
    let router = caps(&["upload"], &["photo"]).route(
        "/uploads/check",
        post(
            |axum::Json(check): axum::Json<serde_json::Value>| async move {
                let existing: Vec<_> = check["checksums"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|sum| sum.as_str().unwrap().ends_with('0'))
                    .map(|sum| json!({"checksum": sum, "item_id": format!("item-{sum}")}))
                    .collect();
                axum::Json(json!({"existing": existing}))
            },
        ),
    );
    let (endpoint, seen, _served) = serve(router).await;
    let dir = tempfile::tempdir().unwrap();
    let (providers, _) = providers(&dir, vec![(photos(), endpoint)]);
    let route = providers
        .route("alice", anyone, &MediaOperation::Upload, None, None)
        .await
        .unwrap();
    let sums: Vec<String> = (0..1005).map(|n| format!("sha1:{n:04}")).collect();
    let existing = route.client.check_uploads("alice", &sums).await.unwrap();
    assert_eq!(existing.len(), 101);
    assert_eq!(existing.last().unwrap().checksum, "sha1:1000");
    let checks = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|(path, _, _)| path == "POST /uploads/check")
        .count();
    assert_eq!(checks, 2);
}

#[tokio::test]
async fn failures_keep_the_providers_code_or_follow_the_status() {
    let router = caps(&["upload"], &["photo"]).route(
        "/uploads",
        post(|headers: HeaderMap| async move {
            match headers
                .get(WONTON_USER_HEADER)
                .and_then(|v| v.to_str().ok())
            {
                Some("gone") => (
                    StatusCode::NOT_FOUND,
                    axum::Json(json!({"code": "server_storage_full", "message": "盘满"})),
                )
                    .into_response(),
                Some("busy") => (
                    StatusCode::TOO_MANY_REQUESTS,
                    [(header::RETRY_AFTER, "30")],
                    "slow",
                )
                    .into_response(),
                Some("broken") => StatusCode::BAD_GATEWAY.into_response(),
                _ => (StatusCode::OK, "not json").into_response(),
            }
        }),
    );
    let (endpoint, _, _served) = serve(router).await;
    let dir = tempfile::tempdir().unwrap();
    let (providers, _) = providers(&dir, vec![(photos(), endpoint)]);
    let attempt = async |person: &str| -> MediaFailure {
        let route = providers
            .route(person, anyone, &MediaOperation::Upload, None, None)
            .await
            .unwrap();
        route
            .client
            .start_upload(person, &upload())
            .await
            .unwrap_err()
    };
    assert_eq!(
        attempt("gone").await.code,
        MediaFailureCode::ServerStorageFull
    );
    let busy = attempt("busy").await;
    assert_eq!(busy.code, MediaFailureCode::RateLimited);
    assert_eq!(busy.retry_after_secs, Some(30));
    assert_eq!(
        attempt("broken").await.code,
        MediaFailureCode::UpstreamError
    );
    assert_eq!(attempt("odd").await.code, MediaFailureCode::Protocol);
}

#[tokio::test]
async fn a_provider_that_does_not_answer_is_unavailable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead = Endpoint::Tcp(listener.local_addr().unwrap().to_string());
    drop(listener);
    let dir = tempfile::tempdir().unwrap();
    let (providers, _) = providers(&dir, vec![(photos(), dead)]);
    let failure = providers
        .route("alice", anyone, &MediaOperation::Upload, None, None)
        .await
        .unwrap_err();
    assert_eq!(failure.code, MediaFailureCode::PluginUnavailable);
}

#[tokio::test]
async fn revoking_a_person_tells_the_provider_and_forgets_the_key() {
    let router =
        caps(&["upload"], &["photo"]).route("/revoke", post(|| async { StatusCode::NO_CONTENT }));
    let (endpoint, seen, _served) = serve(router).await;
    let dir = tempfile::tempdir().unwrap();
    let (providers, book) = providers(&dir, vec![(photos(), endpoint)]);
    providers
        .route("alice", anyone, &MediaOperation::Upload, None, None)
        .await
        .unwrap();
    providers.revoke_person("alice").await.unwrap();
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(path, _, user)| path == "POST /revoke" && user.as_deref() == Some("alice"))
    );
    assert_eq!(
        *book.revoked.lock().unwrap(),
        [("app.example.photos".to_owned(), credential_name("alice"))]
    );
    providers
        .route("alice", anyone, &MediaOperation::Upload, None, None)
        .await
        .unwrap();
    assert_eq!(book.issued.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn several_fitting_providers_need_a_source_or_the_owners_default() {
    let (photos_at, _, _p) = serve(caps(&["download"], &["photo", "video"])).await;
    let (video_at, _, _v) = serve(caps(&["download"], &["video"])).await;
    let dir = tempfile::tempdir().unwrap();
    let ops = [MediaOperation::Download];
    let photos = candidate(
        "app.example.photos",
        &ops,
        &[MediaKind::Photo, MediaKind::Video],
    );
    let mut video = candidate("app.example.video", &ops, &[MediaKind::Video]);
    let entries = |video: &Candidate| {
        vec![
            (photos.clone(), photos_at.clone()),
            (video.clone(), video_at.clone()),
        ]
    };
    let pick = async |video: &Candidate, source: Option<&str>| {
        let (providers, _) = providers(&dir, entries(video));
        providers
            .route("alice", anyone, &ops[0], Some(&MediaKind::Video), source)
            .await
            .map(|route| route.provider)
    };
    // 两家都能下视频、没定默认：不替人挑。
    let failure = pick(&video, None).await.unwrap_err();
    assert_eq!(failure.code, MediaFailureCode::SourceRequired);
    assert_eq!(
        pick(&video, Some("app.example.photos")).await.unwrap(),
        "app.example.photos"
    );
    // 点名的那家做不了，不改去别家。
    let failure = pick(&video, Some("app.example.music")).await.unwrap_err();
    assert_eq!(failure.code, MediaFailureCode::PluginUnavailable);
    video.default = true;
    assert_eq!(pick(&video, None).await.unwrap(), "app.example.video");
    // 照片只有一家能下，默认管不着它。
    let (providers, _) = providers(&dir, entries(&video));
    let only = providers
        .route("alice", anyone, &ops[0], Some(&MediaKind::Photo), None)
        .await
        .unwrap();
    assert_eq!(only.provider, "app.example.photos");
}

#[tokio::test]
async fn a_provider_the_person_cannot_use_does_not_count() {
    let (photos_at, _, _p) = serve(caps(&["download"], &["video"])).await;
    let (video_at, _, _v) = serve(caps(&["download"], &["video"])).await;
    let dir = tempfile::tempdir().unwrap();
    let ops = [MediaOperation::Download];
    let (providers, _) = providers(
        &dir,
        vec![
            (
                candidate("app.example.photos", &ops, &[MediaKind::Video]),
                photos_at,
            ),
            (
                candidate("app.example.video", &ops, &[MediaKind::Video]),
                video_at,
            ),
        ],
    );
    // 两家都能下视频，这个人只能用其中一家：就用它，不回 source_required。
    let only_video = |plugin: &str| plugin == "app.example.video";
    let picked = providers
        .route("alice", only_video, &ops[0], Some(&MediaKind::Video), None)
        .await
        .unwrap();
    assert_eq!(picked.provider, "app.example.video");
    // 能做的都用不了：权限问题，不是“请选一家”。
    let failure = providers
        .route("alice", |_| false, &ops[0], Some(&MediaKind::Video), None)
        .await
        .unwrap_err();
    assert_eq!(failure.code, MediaFailureCode::PermissionDenied);
}
