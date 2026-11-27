//! 离线下载的行为测试：对面是本机的假核心，断点续传、源变了、半截接不上、中途断、上游报错，看最后落下的文件
//! 和报给核心的完成、失败。

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use tokio::net::TcpListener;
use url::Url;
use wonton_protocol::media::{
    MediaFailure, MediaFailureCode, MediaItemRef, MediaKind, MediaTask, Progress, ProgressUnit,
    TaskId, TaskKind, TaskState, TaskTarget, new_task_id,
};

use super::{Downloads, OfflineState, Record};
use crate::media_link::api::MediaApi;

const CONTENT: &[u8] = b"0123456789";
const DEVICE: &str = "this-device";

/// 假核心怎么回字节。
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// 认 `Range`/`If-Range`，ETag 是 `etag`。
    Honest { etag: &'static str },
    /// 不管要哪段，回 206 从 2 开始。
    WrongStart,
    /// 回 200、长度说 10，只给 4 个字节就断。
    CutAt4,
    /// 回媒体接口的错误信封。
    Gone,
}

#[derive(Default)]
struct Seen {
    ranges: Vec<Option<String>>,
    completed: u32,
    failures: Vec<MediaFailure>,
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
    core.seen.lock().unwrap().ranges.push(header("range"));
    let mode = *core.mode.lock().unwrap();
    match mode {
        Mode::Honest { etag } => {
            let start = header("range")
                .and_then(|r| {
                    r.strip_prefix("bytes=")?
                        .split('-')
                        .next()?
                        .parse::<usize>()
                        .ok()
                })
                .filter(|_| header("if-range").is_none_or(|tag| tag == etag));
            match start {
                Some(start) => (
                    StatusCode::PARTIAL_CONTENT,
                    [
                        ("content-range", format!("bytes {start}-9/10")),
                        ("etag", etag.to_owned()),
                        ("content-type", "video/mp4".to_owned()),
                    ],
                    CONTENT[start..].to_vec(),
                )
                    .into_response(),
                None => (
                    [
                        ("etag", etag.to_owned()),
                        ("content-type", "video/mp4".to_owned()),
                    ],
                    CONTENT.to_vec(),
                )
                    .into_response(),
            }
        }
        Mode::WrongStart => (
            StatusCode::PARTIAL_CONTENT,
            [("content-range", "bytes 2-9/10"), ("etag", "\"v1\"")],
            CONTENT[2..].to_vec(),
        )
            .into_response(),
        Mode::CutAt4 => {
            // 先让头和前 4 个字节发出去再断：紧接着出错时头可能还在缓冲里，客户端只看到连接断了。
            let chunks = futures_util::stream::unfold(0, |step| async move {
                match step {
                    0 => Some((Ok(Bytes::from_static(&CONTENT[..4])), 1)),
                    1 => {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        Some((Err(std::io::Error::other("cut")), 2))
                    }
                    _ => None,
                }
            });
            (
                [("content-length", "10"), ("etag", "\"v1\"")],
                Body::from_stream(chunks),
            )
                .into_response()
        }
        Mode::Gone => (
            StatusCode::NOT_FOUND,
            [("content-type", "application/json")],
            r#"{"code":"media_gone","message":"上游删了"}"#,
        )
            .into_response(),
    }
}

async fn report(
    State(core): State<Core>,
    Path((id, action)): Path<(TaskId, String)>,
    body: Bytes,
) -> Response {
    let mut seen = core.seen.lock().unwrap();
    match action.as_str() {
        "complete" => seen.completed += 1,
        "failure" => {
            let failure: MediaFailure = serde_json::from_slice(&body).unwrap();
            seen.failures.push(failure.clone());
            // 和核心一样：传输类排进等待、到点再叫，其余的失败。
            let state = if failure.code == MediaFailureCode::TransportUnavailable {
                TaskState::WaitingNetwork
            } else {
                TaskState::Failed(failure)
            };
            let mut reported = task(state, DEVICE);
            reported.id = id;
            return axum::Json(reported).into_response();
        }
        _ => {}
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn fake_core(mode: Mode) -> (MediaApi, Core) {
    let core = Core {
        mode: Arc::new(Mutex::new(mode)),
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

/// 上次下到一半：半截里有 `have` 个字节，记着 `etag`。
fn half_done(downloads: &Downloads, task: TaskId, have: usize, etag: Option<&str>) {
    downloads
        .write_record(&Record {
            task,
            item: item(),
            etag: etag.map(str::to_owned),
            total: Some(10),
            file: None,
        })
        .unwrap();
    std::fs::write(downloads.part(task), &CONTENT[..have]).unwrap();
}

fn ready_bytes(downloads: &Downloads) -> Vec<u8> {
    match downloads.offline("ep/1") {
        Some(OfflineState::Ready { path }) => std::fs::read(path).unwrap(),
        other => panic!("not ready: {other:?}"),
    }
}

#[tokio::test]
async fn a_fresh_download_lands_as_a_playable_file_and_is_reported_done() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let (api, core) = fake_core(Mode::Honest { etag: "\"v1\"" }).await;
    let task = new_task_id();
    let path = downloads.run(&api, task, item()).await.unwrap();
    assert_eq!(path.extension().unwrap(), "mp4");
    assert_eq!(ready_bytes(&downloads), CONTENT);
    assert!(!downloads.part(task).exists());
    assert_eq!(core.seen.lock().unwrap().completed, 1);
}

#[tokio::test]
async fn an_interrupted_download_continues_from_where_it_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let (api, core) = fake_core(Mode::Honest { etag: "\"v1\"" }).await;
    let task = new_task_id();
    half_done(&downloads, task, 4, Some("\"v1\""));
    downloads.run(&api, task, item()).await.unwrap();
    assert_eq!(ready_bytes(&downloads), CONTENT);
    assert_eq!(
        core.seen.lock().unwrap().ranges,
        vec![Some("bytes=4-".into())]
    );
}

#[tokio::test]
async fn a_changed_source_starts_over_instead_of_splicing() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let (api, _core) = fake_core(Mode::Honest { etag: "\"v2\"" }).await;
    let task = new_task_id();
    // 半截是旧版本的字节：接上去就是坏文件。
    std::fs::create_dir_all(&downloads.dir).unwrap();
    half_done(&downloads, task, 4, Some("\"v1\""));
    std::fs::write(downloads.part(task), b"XXXX").unwrap();
    downloads.run(&api, task, item()).await.unwrap();
    assert_eq!(ready_bytes(&downloads), CONTENT);
}

#[tokio::test]
async fn without_an_etag_nothing_is_resumed() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let (api, core) = fake_core(Mode::Honest { etag: "\"v1\"" }).await;
    let task = new_task_id();
    half_done(&downloads, task, 4, None);
    downloads.run(&api, task, item()).await.unwrap();
    assert_eq!(ready_bytes(&downloads), CONTENT);
    assert_eq!(core.seen.lock().unwrap().ranges, vec![None]);
}

#[tokio::test]
async fn bytes_that_do_not_line_up_are_dropped_and_fetched_again_from_zero() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let (api, core) = fake_core(Mode::WrongStart).await;
    let task = new_task_id();
    half_done(&downloads, task, 4, Some("\"v1\""));
    let failure = downloads.run(&api, task, item()).await.unwrap_err();
    // 从 0 再要一次还是接不上：不硬拼，报给核心。
    assert_eq!(failure.code, MediaFailureCode::Protocol);
    let seen = core.seen.lock().unwrap();
    assert_eq!(seen.ranges, vec![Some("bytes=4-".into()), None]);
    assert_eq!(seen.failures.len(), 1);
    assert!(
        downloads
            .offline("ep/1")
            .is_none_or(|s| !matches!(s, OfflineState::Ready { .. }))
    );
}

#[tokio::test]
async fn a_cut_connection_keeps_the_part_for_next_time() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let (api, core) = fake_core(Mode::CutAt4).await;
    let task = new_task_id();
    let failure = downloads.run(&api, task, item()).await.unwrap_err();
    assert_eq!(failure.code, MediaFailureCode::TransportUnavailable);
    // 断了就不再是“在下”：核心排进等待，壳看到的是等着接着下和这次断的原因。
    assert_eq!(
        downloads.offline("ep/1"),
        Some(OfflineState::Waiting {
            done: 4,
            total: Some(10),
            failure: Some(failure),
        })
    );

    *core.mode.lock().unwrap() = Mode::Honest { etag: "\"v1\"" };
    downloads.run(&api, task, item()).await.unwrap();
    assert_eq!(ready_bytes(&downloads), CONTENT);
    assert_eq!(
        core.seen.lock().unwrap().ranges.last().unwrap().as_deref(),
        Some("bytes=4-")
    );
}

#[tokio::test]
async fn an_upstream_failure_is_reported_with_its_category() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let (api, core) = fake_core(Mode::Gone).await;
    // 一个字节都没下到也有离线副本：壳要显示失败和原因，不是又回到“下载”。
    let failure = downloads
        .run(&api, new_task_id(), item())
        .await
        .unwrap_err();
    assert_eq!(failure.code, MediaFailureCode::MediaGone);
    assert_eq!(
        core.seen.lock().unwrap().failures[0].code,
        MediaFailureCode::MediaGone
    );
    assert_eq!(
        downloads.offline("ep/1"),
        Some(OfflineState::Failed { failure })
    );
}

#[tokio::test]
async fn a_retried_download_continues_from_the_failed_tasks_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let (api, core) = fake_core(Mode::Gone).await;
    let old = new_task_id();
    half_done(&downloads, old, 4, Some("\"v1\""));
    downloads.run(&api, old, item()).await.unwrap_err();
    assert_eq!(downloads.failed_task("ep/1"), Some(old));

    let new = new_task_id();
    downloads.adopt(old, new).unwrap();
    assert_eq!(downloads.failed_task("ep/1"), None);
    *core.mode.lock().unwrap() = Mode::Honest { etag: "\"v1\"" };
    downloads.run(&api, new, item()).await.unwrap();
    assert_eq!(ready_bytes(&downloads), CONTENT);
    assert_eq!(
        core.seen.lock().unwrap().ranges.last().unwrap().as_deref(),
        Some("bytes=4-")
    );
    assert_eq!(downloads.records().len(), 1);
}

fn task(state: TaskState, device: &str) -> MediaTask {
    MediaTask {
        id: new_task_id(),
        kind: TaskKind::Download,
        person: "@alice:example.com".into(),
        device: device.into(),
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

#[tokio::test]
async fn only_this_devices_live_downloads_run_and_cancelled_ones_are_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let (api, core) = fake_core(Mode::Honest { etag: "\"v1\"" }).await;
    let cancelled = task(TaskState::Cancelled, DEVICE);
    half_done(&downloads, cancelled.id, 4, Some("\"v1\""));
    let tasks = vec![
        task(TaskState::Running, "another-phone"),
        task(TaskState::WaitingNetwork, DEVICE),
        cancelled.clone(),
    ];
    downloads.reconcile(&tokio::runtime::Handle::current(), &api, DEVICE, &tasks);
    assert!(!downloads.part(cancelled.id).exists());
    assert!(downloads.records().is_empty());

    let live = task(TaskState::Queued, DEVICE);
    downloads.reconcile(&tokio::runtime::Handle::current(), &api, DEVICE, &[live]);
    for _ in 0..200 {
        if core.seen.lock().unwrap().completed == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(ready_bytes(&downloads), CONTENT);
    assert_eq!(core.seen.lock().unwrap().ranges.len(), 1);
}

#[tokio::test]
async fn removing_an_offline_copy_deletes_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = Downloads::open(dir.path());
    let (api, _core) = fake_core(Mode::Honest { etag: "\"v1\"" }).await;
    downloads.run(&api, new_task_id(), item()).await.unwrap();
    downloads.remove("ep/1").unwrap();
    assert!(downloads.offline("ep/1").is_none());
    assert_eq!(files_under(&downloads.dir), 0);
}

fn files_under(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| {
            if e.path().is_dir() {
                files_under(&e.path())
            } else {
                1
            }
        })
        .sum()
}
