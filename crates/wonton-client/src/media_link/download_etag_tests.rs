//! 失败任务重试接上半截后，源变了（ETag 变了或没有）要从 0 来，不能把旧头接新尾；服务端不理 If-Range 回 206 时也一样。
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use tokio::net::TcpListener;
use url::Url;
use wonton_protocol::media::{
    MediaFailure, MediaItemRef, MediaKind, MediaTask, Progress, ProgressUnit, TaskId, TaskKind,
    TaskState, TaskTarget, new_task_id,
};

use super::{Downloads, OfflineState, Record};
use crate::media_link::api::MediaApi;

const OLD: &[u8] = b"XXXX456789"; // 旧源：前 4 个字节是 XXXX
const NEW: &[u8] = b"ABCDEFGHIJ"; // 新源
const DEVICE: &str = "this-device";

#[derive(Clone, Copy)]
enum Mode {
    Gone,
    /// 源换了，老实回 200。
    Changed,
    /// 源换了，却按 Range 回 206（忽略 If-Range），ETag 是没有或者是另一个。
    ChangedWithRange(Option<&'static str>),
}

#[derive(Default)]
struct Seen {
    ranges: Vec<Option<String>>,
    if_ranges: Vec<Option<String>>,
}

#[derive(Clone)]
struct Core {
    mode: Arc<Mutex<Mode>>,
    seen: Arc<Mutex<Seen>>,
}

async fn content(State(core): State<Core>, headers: HeaderMap) -> Response {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    {
        let mut seen = core.seen.lock().unwrap();
        seen.ranges.push(header("range"));
        seen.if_ranges.push(header("if-range"));
    }
    match *core.mode.lock().unwrap() {
        Mode::Gone => (
            StatusCode::NOT_FOUND,
            [("content-type", "application/json")],
            r#"{"code":"media_gone","message":"gone"}"#,
        )
            .into_response(),
        // 源换了：ETag 是 v2，拿 v1 来续的回 200 整份。
        Mode::Changed => (
            [("etag", "\"v2\""), ("content-type", "video/mp4")],
            NEW.to_vec(),
        )
            .into_response(),
        Mode::ChangedWithRange(etag) => {
            let start = header("range")
                .and_then(|r| {
                    r.strip_prefix("bytes=")?
                        .split('-')
                        .next()?
                        .parse::<usize>()
                        .ok()
                })
                .unwrap_or(0);
            let mut response = Response::new(axum::body::Body::from(NEW[start..].to_vec()));
            *response.status_mut() = StatusCode::PARTIAL_CONTENT;
            let headers = response.headers_mut();
            headers.insert("content-type", "video/mp4".parse().unwrap());
            headers.insert(
                "content-range",
                format!("bytes {start}-9/10").parse().unwrap(),
            );
            if let Some(etag) = etag {
                headers.insert("etag", etag.parse().unwrap());
            }
            response
        }
    }
}

fn task(state: TaskState) -> MediaTask {
    MediaTask {
        id: new_task_id(),
        kind: TaskKind::Download,
        person: "p".into(),
        device: DEVICE.into(),
        target: TaskTarget::Download(item()),
        state,
        progress: Progress {
            done: 0,
            total: None,
            unit: ProgressUnit::Bytes,
        },
        retry_count: 0,
        retry_of: None,
        next_attempt_at: None,
        created_at: 1,
        partial_failures: Vec::new(),
    }
}

async fn report(
    State(_core): State<Core>,
    axum::extract::Path((id, action)): Path<(TaskId, String)>,
    body: axum::body::Bytes,
) -> Response {
    if action == "failure" {
        let failure: MediaFailure = serde_json::from_slice(&body).unwrap();
        let mut reported = task(TaskState::Failed(failure));
        reported.id = id;
        return axum::Json(reported).into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn fake_core() -> (MediaApi, Core) {
    let core = Core {
        mode: Arc::new(Mutex::new(Mode::Gone)),
        seen: Arc::default(),
    };
    let app = Router::new()
        .route(
            "/_wonton/api/media/tasks/{task}/items/{item}/content",
            get(content),
        )
        .route("/_wonton/api/media/tasks/{task}/{action}", post(report))
        .with_state(core.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    (MediaApi::new(base, client, DEVICE.into()), core)
}

fn item() -> MediaItemRef {
    MediaItemRef {
        id: "ep/1".into(),
        kind: MediaKind::Video,
        title: "第一集".into(),
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

#[tokio::test]
async fn a_retry_onto_a_changed_source_restarts_from_zero() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let (api, core) = fake_core().await;
    let old = new_task_id();
    downloads
        .write_record(&Record {
            task: old,
            item: item(),
            etag: Some("\"v1\"".into()),
            total: Some(OLD.len() as u64),
            file: None,
        })
        .unwrap();
    std::fs::write(downloads.part(old), &OLD[..4]).unwrap();

    downloads.run(&api, old, item()).await.unwrap_err();
    assert!(matches!(
        downloads.offline("ep/1"),
        Some(OfflineState::Failed { .. })
    ));
    assert_eq!(downloads.failed_task("ep/1"), Some(old));

    let new = new_task_id();
    downloads.adopt(old, new).unwrap();
    *core.mode.lock().unwrap() = Mode::Changed;
    let before = core.seen.lock().unwrap().ranges.len();
    downloads.run(&api, new, item()).await.unwrap();

    let got = match downloads.offline("ep/1") {
        Some(OfflineState::Ready { path }) => std::fs::read(path).unwrap(),
        other => panic!("{other:?}"),
    };
    let seen = core.seen.lock().unwrap();
    assert_eq!(
        seen.ranges[before],
        Some("bytes=4-".into()),
        "接续请求要带 Range"
    );
    assert_eq!(
        seen.if_ranges[before].as_deref(),
        Some("\"v1\""),
        "接续要带旧 ETag"
    );
    assert_eq!(got, NEW.to_vec(), "源变了要从 0 来，不能拼旧头");
    assert_eq!(downloads.records().len(), 1);
}

async fn retried_changed_source(etag: Option<&'static str>) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let (api, core) = fake_core().await;
    let old = new_task_id();
    downloads
        .write_record(&Record {
            task: old,
            item: item(),
            etag: Some("\"v1\"".into()),
            total: Some(OLD.len() as u64),
            file: None,
        })
        .unwrap();
    std::fs::write(downloads.part(old), &OLD[..4]).unwrap();
    downloads.run(&api, old, item()).await.unwrap_err();
    let new = new_task_id();
    downloads.adopt(old, new).unwrap();
    *core.mode.lock().unwrap() = Mode::ChangedWithRange(etag);
    downloads.run(&api, new, item()).await.unwrap();
    match downloads.offline("ep/1") {
        Some(OfflineState::Ready { path }) => std::fs::read(path).unwrap(),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_retry_onto_a_changed_source_without_an_etag_restarts() {
    let got = retried_changed_source(None).await;
    assert_eq!(
        String::from_utf8_lossy(&got),
        "ABCDEFGHIJ",
        "没有 ETag 的 206 不能拼在旧头后面"
    );
}

#[tokio::test]
async fn a_retry_onto_a_changed_source_with_another_etag_restarts() {
    let got = retried_changed_source(Some("\"v2\"")).await;
    assert_eq!(
        String::from_utf8_lossy(&got),
        "ABCDEFGHIJ",
        "206 带回另一个 ETag 也不能拼"
    );
}
