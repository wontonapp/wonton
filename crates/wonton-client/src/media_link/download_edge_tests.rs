//! 下载执行层的边角：源没被要 Range 也回 206、换了源但这一轮打不开半截、扩展名和记录撞名、写完的文件被外面删了。
use std::os::unix::fs::PermissionsExt;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use tokio::net::TcpListener;
use url::Url;
use wonton_protocol::media::{MediaFailureCode, MediaItemRef, MediaKind, new_task_id};

use super::{Downloads, OfflineState, Record};
use crate::media_link::api::MediaApi;

const CONTENT: &[u8] = b"0123456789";
const DEVICE: &str = "this-device";

#[derive(Clone, Copy)]
enum Mode {
    /// 源换了一份：只认 `v2`，拿 `v1` 来续的回 200 整份。
    Changed,
    /// 不管要不要 Range，都回 206 从 0 起的整份。
    AlwaysPartial,
    /// 提供方说这是 JSON 文件（扩展名和记录文件一样）。
    Json,
}

#[derive(Clone)]
struct Core {
    mode: Mode,
}

async fn content(State(core): State<Core>, headers: HeaderMap) -> axum::response::Response {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    match core.mode {
        Mode::Json => (
            [
                ("etag", "\"v1\"".to_owned()),
                ("content-type", "application/json".to_owned()),
                (
                    "content-disposition",
                    "attachment; filename=\"show.json\"".to_owned(),
                ),
            ],
            CONTENT.to_vec(),
        )
            .into_response(),
        Mode::AlwaysPartial => (
            StatusCode::PARTIAL_CONTENT,
            [
                ("content-range", "bytes 0-9/10".to_owned()),
                ("etag", "\"v1\"".to_owned()),
                ("content-type", "video/mp4".to_owned()),
            ],
            CONTENT.to_vec(),
        )
            .into_response(),
        Mode::Changed => {
            let start = header("range")
                .and_then(|r| {
                    r.strip_prefix("bytes=")?
                        .split('-')
                        .next()?
                        .parse::<usize>()
                        .ok()
                })
                .filter(|_| header("if-range").is_none_or(|tag| tag == "\"v2\""));
            match start {
                Some(start) => (
                    StatusCode::PARTIAL_CONTENT,
                    [
                        ("content-range", format!("bytes {start}-9/10")),
                        ("etag", "\"v2\"".to_owned()),
                        ("content-type", "video/mp4".to_owned()),
                    ],
                    CONTENT[start..].to_vec(),
                )
                    .into_response(),
                None => (
                    [
                        ("etag", "\"v2\"".to_owned()),
                        ("content-type", "video/mp4".to_owned()),
                    ],
                    CONTENT.to_vec(),
                )
                    .into_response(),
            }
        }
    }
}

async fn report() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn fake_core(mode: Mode) -> MediaApi {
    let app = Router::new()
        .route(
            "/_wonton/api/media/tasks/{task}/items/{item}/content",
            get(content),
        )
        .route("/_wonton/api/media/tasks/{task}/{action}", post(report))
        .with_state(Core { mode });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    MediaApi::new(base, client, DEVICE.into())
}

fn item() -> MediaItemRef {
    MediaItemRef {
        id: "ep/1".into(),
        kind: MediaKind::Video,
        title: "Episode 1".into(),
        artist: None,
        album: None,
        artwork_url: None,
        duration_ms: None,
        series: None,
        index: None,
        played: false,
        starred: false,
        source: None,
    }
}

fn ready_bytes(downloads: &Downloads) -> Vec<u8> {
    match downloads.offline("ep/1") {
        Some(OfflineState::Ready { path }) => std::fs::read(path).unwrap(),
        other => panic!("not ready: {other:?}"),
    }
}

#[tokio::test]
async fn a_partial_response_to_a_fresh_request_is_the_whole_file() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let api = fake_core(Mode::AlwaysPartial).await;
    let path = downloads.run(&api, new_task_id(), item()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), CONTENT);
}

#[tokio::test]
async fn a_changed_source_that_cannot_be_written_now_is_not_spliced_later() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let task = new_task_id();
    downloads
        .write_record(&Record {
            task,
            item: item(),
            etag: Some("\"v1\"".into()),
            total: Some(10),
            file: None,
        })
        .unwrap();
    let part = downloads.part(task);
    std::fs::write(&part, b"XXXX").unwrap();
    std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o444)).unwrap();

    let api = fake_core(Mode::Changed).await;
    let failure = downloads.run(&api, task, item()).await.unwrap_err();
    assert_eq!(failure.code, MediaFailureCode::LocalPermissionDenied);

    std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o644)).unwrap();
    downloads.run(&api, task, item()).await.unwrap();
    assert_eq!(ready_bytes(&downloads), CONTENT);
}

#[tokio::test]
async fn a_file_named_like_a_record_keeps_its_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let api = fake_core(Mode::Json).await;
    let path = downloads.run(&api, new_task_id(), item()).await.unwrap();
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("json"));
    assert_eq!(std::fs::read(&path).unwrap(), CONTENT);
    assert_eq!(ready_bytes(&downloads), CONTENT);
}

#[tokio::test]
async fn a_finished_file_deleted_from_outside_is_downloaded_again() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let api = fake_core(Mode::AlwaysPartial).await;
    let task = new_task_id();
    let path = downloads.run(&api, task, item()).await.unwrap();
    std::fs::remove_file(&path).unwrap();

    let again = downloads.run(&api, task, item()).await.unwrap();
    assert_eq!(std::fs::read(again).unwrap(), CONTENT);
    assert_eq!(ready_bytes(&downloads), CONTENT);
}
