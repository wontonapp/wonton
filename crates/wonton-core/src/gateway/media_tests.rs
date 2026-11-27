//! 媒体接口的行为测试（ADR-0113 第 2 节“字节路径”）：进程内调网关，设备身份放进请求扩展；提供方是本机
//! 的假 HTTP 服务，按契约记偏移、回 206。

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use hyper::{Method, Request};
use serde_json::{Value, json};
use wonton_protocol::media::api::{
    session_item_stream_path, session_stream_path, task_item_content_path, task_upload_path,
};
use wonton_protocol::media::{
    MediaKind, MediaOperation, PlaybackSessionId, ProviderStream, WONTON_DEVICE_HEADER,
    uploads::UPLOAD_OFFSET_HEADER,
};

use super::files_tests::paired_gateway;
use super::tests::CORE;
use super::{TunnelPeer, router};
use crate::media::Media;
use crate::media::test_support::{FakeDirectory, Seen, Served, candidate, caps, serve};

const PROVIDER: &str = "app.example.media";

/// 假提供方的状态：一个上传号对应的已收字节；`forget` 打开后 `HEAD` 一律回 404。
#[derive(Default)]
struct Uploads {
    next: u32,
    received: Vec<(String, Vec<u8>)>,
    forget: bool,
    started: Vec<Value>,
}

type Shared = Arc<Mutex<Uploads>>;

const CONTENT: &[u8] = b"0123456789";

fn range_start(headers: &HeaderMap, name: header::HeaderName) -> Option<usize> {
    let value = headers.get(name)?.to_str().ok()?;
    let spec = value.strip_prefix("bytes")?.trim_start_matches(['=', ' ']);
    spec.split('-').next()?.parse().ok()
}

fn partial(start: usize) -> Response {
    let body = CONTENT[start..].to_vec();
    (
        StatusCode::PARTIAL_CONTENT,
        [(
            header::CONTENT_RANGE,
            format!("bytes {start}-{}/{}", CONTENT.len() - 1, CONTENT.len()),
        )],
        body,
    )
        .into_response()
}

fn provider(shared: Shared) -> Router {
    let (start, head, patch, finish) = (shared.clone(), shared.clone(), shared.clone(), shared);
    caps(&["upload", "download", "playback"], &["photo", "video"])
        .route(
            "/uploads",
            post(move |body: Bytes| async move {
                let mut state = start.lock().unwrap();
                let started: Value = serde_json::from_slice(&body).unwrap();
                if started["path"] == "deny" {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                state.started.push(started);
                state.next += 1;
                let id = format!("u{}", state.next);
                state.received.push((id.clone(), Vec::new()));
                axum::Json(json!({"upload_id": id, "upload_offset": 0})).into_response()
            }),
        )
        .route(
            "/uploads/{id}",
            axum::routing::head(move |Path(id): Path<String>| async move {
                let state = head.lock().unwrap();
                match state.received.iter().find(|(known, _)| *known == id) {
                    Some((_, bytes)) if !state.forget => (
                        StatusCode::OK,
                        [(UPLOAD_OFFSET_HEADER, bytes.len().to_string())],
                    )
                        .into_response(),
                    _ => StatusCode::NOT_FOUND.into_response(),
                }
            })
            .patch(
                move |Path(id): Path<String>, headers: HeaderMap, body: Bytes| async move {
                    let mut state = patch.lock().unwrap();
                    let (_, bytes) = state
                        .received
                        .iter_mut()
                        .find(|(known, _)| *known == id)
                        .unwrap();
                    // 只认 `Upload-Offset`，和真提供方一样（tus 的写法）。
                    let at: usize = headers[UPLOAD_OFFSET_HEADER]
                        .to_str()
                        .unwrap()
                        .parse()
                        .unwrap();
                    if at != bytes.len() {
                        let current = bytes.len();
                        return (
                            StatusCode::CONFLICT,
                            axum::Json(json!({"upload_offset": current})),
                        )
                            .into_response();
                    }
                    bytes.extend_from_slice(&body);
                    axum::Json(json!({"upload_offset": bytes.len()})).into_response()
                },
            ),
        )
        .route(
            "/uploads/{id}/finish",
            post(move |Path(id): Path<String>| async move {
                let state = finish.lock().unwrap();
                let index = state
                    .received
                    .iter()
                    .position(|(known, _)| *known == id)
                    .unwrap();
                let duplicate = state.started[index]["path"] == "dup";
                axum::Json(json!({"item_id": format!("asset-{id}"), "duplicate": duplicate}))
            }),
        )
        .route(
            "/items/{id}/download",
            get(|headers: HeaderMap| async move {
                partial(range_start(&headers, header::RANGE).unwrap_or(0))
            }),
        )
        .route(
            "/playback",
            post(|| async {
                axum::Json(json!({
                    "session": "up-1",
                    "stream": {"direct": "http://upstream.invalid/stream/v1?token=abc"},
                    "origin_stream": null,
                    "segments": [], "audio": [], "subtitles": [],
                    "seekable": true, "duration_ms": 10_000, "expires_at": 0,
                }))
            }),
        )
        .route(
            "/stream/v1",
            get(|headers: HeaderMap| async move {
                partial(range_start(&headers, header::RANGE).unwrap_or(0))
            }),
        )
}

struct Fixture {
    _dir: tempfile::TempDir,
    peer: TunnelPeer,
    app: Router,
    seen: Seen,
    uploads: Shared,
    _served: Served,
}

async fn fixture() -> Fixture {
    let uploads = Shared::default();
    let (endpoint, seen, served) = serve(provider(uploads.clone())).await;
    let dir = tempfile::TempDir::new().unwrap();
    let paired = paired_gateway(dir.path()).await;
    let (directory, _) = FakeDirectory::new(vec![(
        candidate(
            PROVIDER,
            &[
                MediaOperation::Upload,
                MediaOperation::Download,
                MediaOperation::Playback,
            ],
            &[MediaKind::Photo, MediaKind::Video],
        ),
        endpoint,
    )]);
    let media = Media::with_directory(
        &dir.path().join("media"),
        Arc::new(directory),
        crate::http::HttpClient::new(),
    )
    .unwrap();
    let app = router(Arc::new(paired.gateway.with_media(Some(Arc::new(media)))));
    Fixture {
        _dir: dir,
        peer: paired.device,
        app,
        seen,
        uploads,
        _served: served,
    }
}

impl Fixture {
    fn device(&self) -> String {
        crate::files::device_label(&self.peer.0)
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, String)],
        body: Body,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, format!("{CORE}:18480"))
            .header(WONTON_DEVICE_HEADER, self.device())
            .extension(self.peer);
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        let response = tower::ServiceExt::oneshot(self.app.clone(), request.body(body).unwrap())
            .await
            .unwrap();
        let (status, headers) = (response.status(), response.headers().clone());
        let body = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        (status, headers, body)
    }

    async fn json(&self, method: Method, path: &str, body: Value) -> (StatusCode, Value) {
        let (status, _, bytes) = self
            .send(
                method,
                path,
                &[("content-type", "application/json".into())],
                Body::from(body.to_string()),
            )
            .await;
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, value)
    }

    async fn upload_task(&self, path: &str) -> String {
        let (status, task) = self
            .json(
                Method::POST,
                "/_wonton/api/media/tasks",
                json!({
                    "kind": "upload", "person": "", "device": "",
                    "target": {"kind": "upload", "value": [{
                        "item_id": "i1", "path": path, "size": 10, "checksum": null,
                        "taken_at": null, "state": {"state": "queued"}, "upload_offset": 0,
                    }]},
                    "options": {"wifi_only": false, "charging_only": false, "max_bitrate": null,
                                "albums": [], "force_now": false},
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{task}");
        task["id"].as_str().unwrap().to_owned()
    }

    async fn patch(&self, task: &str, range: &str, bytes: &'static [u8]) -> (StatusCode, Value) {
        let path = task_upload_path(task.parse().unwrap(), "i1");
        let (status, _, body) = self
            .send(
                Method::PATCH,
                &path,
                &[("content-range", range.into())],
                Body::from(bytes),
            )
            .await;
        (status, serde_json::from_slice(&body).unwrap())
    }

    async fn start_session(&self) -> (PlaybackSessionId, Value) {
        let f = self;
        let (status, session) = f
        .json(
            Method::POST,
            "/_wonton/api/media/sessions",
            json!({
                "position_ms": 0, "audio": null, "subtitle": null, "device": "", "person": "",
                "item": {"id": "v1", "kind": "video", "title": "海边", "artist": null,
                         "album": null, "artwork_url": null, "duration_ms": 10000, "series": null,
                         "index": null, "played": false, "starred": false},
                "profile": {"direct_play": ["video/mp4"], "max_bitrate": 8000000, "hdr": false,
                            "container": ["mp4"]},
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{session}");
        (session["id"].as_str().unwrap().parse().unwrap(), session)
    }

    fn hits(&self, prefix: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _, _)| target.starts_with(prefix))
            .count()
    }
}

#[tokio::test]
async fn upload_opens_once_resumes_from_the_offset_and_finishes() {
    let f = fixture().await;
    let task = f.upload_task("DCIM/1.jpg").await;
    let start = task_upload_path(task.parse().unwrap(), "i1");
    let (status, offset) = f.json(Method::POST, &start, Value::Null).await;
    assert_eq!(
        (status, offset["upload_offset"].as_u64()),
        (StatusCode::OK, Some(0))
    );
    assert_eq!(f.uploads.lock().unwrap().started[0]["path"], "DCIM/1.jpg");

    let (status, offset) = f.patch(&task, "bytes 0-4/10", b"01234").await;
    assert_eq!(
        (status, offset["upload_offset"].as_u64()),
        (StatusCode::OK, Some(5))
    );
    // 再开一次：核心按记下的号查断点，不重开。
    let (_, offset) = f.json(Method::POST, &start, Value::Null).await;
    assert_eq!(offset["upload_offset"].as_u64(), Some(5));
    assert_eq!(f.hits("POST /uploads"), 1);
    assert_eq!(f.hits("HEAD /uploads/u1"), 1);

    let (status, offset) = f.patch(&task, "bytes 0-4/10", b"01234").await;
    assert_eq!(
        (status, offset["upload_offset"].as_u64()),
        (StatusCode::CONFLICT, Some(5))
    );

    let (status, _) = f.patch(&task, "bytes 5-9/10", b"56789").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(f.hits("POST /uploads/u1/finish"), 1);
    let (_, read) = f
        .json(
            Method::GET,
            &format!("/_wonton/api/media/tasks/{task}"),
            Value::Null,
        )
        .await;
    assert_eq!(read["state"], "done");
    assert_eq!(read["progress"]["done"], 10);
}

#[tokio::test]
async fn a_duplicate_found_at_finish_is_skipped_not_failed() {
    let f = fixture().await;
    let task = f.upload_task("dup").await;
    let start = task_upload_path(task.parse().unwrap(), "i1");
    f.json(Method::POST, &start, Value::Null).await;
    let (status, _) = f.patch(&task, "bytes 0-9/10", CONTENT).await;
    assert_eq!(status, StatusCode::OK);
    let (_, read) = f
        .json(
            Method::GET,
            &format!("/_wonton/api/media/tasks/{task}"),
            Value::Null,
        )
        .await;
    assert_eq!(read["state"], "done");
    assert_eq!(read["target"]["value"][0]["state"]["state"], "skipped");
}

#[tokio::test]
async fn an_upload_the_provider_forgot_is_opened_again() {
    let f = fixture().await;
    let task = f.upload_task("DCIM/2.jpg").await;
    let start = task_upload_path(task.parse().unwrap(), "i1");
    f.json(Method::POST, &start, Value::Null).await;
    f.patch(&task, "bytes 0-4/10", b"01234").await;
    f.uploads.lock().unwrap().forget = true;
    let (_, offset) = f.json(Method::POST, &start, Value::Null).await;
    assert_eq!(offset["upload_offset"].as_u64(), Some(0));
    assert_eq!(f.hits("POST /uploads"), 2);
    // 新号接着用：续写落到 u2 上。
    f.uploads.lock().unwrap().forget = false;
    let (status, offset) = f.patch(&task, "bytes 0-4/10", b"01234").await;
    assert_eq!(
        (status, offset["upload_offset"].as_u64()),
        (StatusCode::OK, Some(5))
    );
    assert_eq!(f.hits("PATCH /uploads/u2"), 1);
}

#[tokio::test]
async fn upstream_401_fails_the_upload_task_without_backoff() {
    let f = fixture().await;
    let task = f.upload_task("deny").await;
    let start = task_upload_path(task.parse().unwrap(), "i1");
    let (_, failure) = f.json(Method::POST, &start, Value::Null).await;
    assert_eq!(failure["code"], "authentication_required");
    let (_, read) = f
        .json(
            Method::GET,
            &format!("/_wonton/api/media/tasks/{task}"),
            Value::Null,
        )
        .await;
    assert_eq!(read["state"]["failed"]["code"], "authentication_required");
    assert_eq!(read["retry_count"], 0);
}

#[tokio::test]
async fn download_bytes_keep_the_range() {
    let f = fixture().await;
    let (status, task) = f
        .json(
            Method::POST,
            "/_wonton/api/media/tasks",
            json!({
                "kind": "download", "person": "", "device": "",
                "target": {"kind": "download", "value": {
                    "id": "v1", "kind": "video", "title": "海边", "artist": null, "album": null,
                    "artwork_url": null, "duration_ms": null, "series": null, "index": null,
                    "played": false, "starred": false,
                }},
                "options": {"wifi_only": false, "charging_only": false, "max_bitrate": null,
                            "albums": [], "force_now": false},
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    let path = task_item_content_path(task["id"].as_str().unwrap().parse().unwrap(), "v1");
    let (status, headers, body) = f
        .send(
            Method::GET,
            &path,
            &[("range", "bytes=3-".into())],
            Body::empty(),
        )
        .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(headers[header::CONTENT_RANGE], "bytes 3-9/10");
    assert_eq!(body, b"3456789");
}

#[tokio::test]
async fn the_session_stream_goes_through_the_core_to_the_provider_path() {
    let f = fixture().await;
    let (id, session) = f.start_session().await;
    let stream: ProviderStream = serde_json::from_value(session["plan"]["stream"].clone()).unwrap();
    assert_eq!(stream, ProviderStream::Direct(session_stream_path(id)));
    assert!(session["plan"]["origin_stream"].is_null());
    let (status, _, body) = f
        .send(
            Method::GET,
            &session_stream_path(id),
            &[("range", "bytes=8-".into())],
            Body::empty(),
        )
        .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(body, b"89");
    assert_eq!(f.hits("GET /stream/v1?token=abc"), 1);
}

#[tokio::test]
async fn only_the_paired_device_itself_gets_in() {
    let f = fixture().await;
    let response = tower::ServiceExt::oneshot(
        f.app.clone(),
        Request::builder()
            .uri("/_wonton/api/media/tasks")
            .header(header::HOST, format!("{CORE}:18480"))
            .extension(f.peer)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let (status, failure) = f
        .json(
            Method::POST,
            "/_wonton/api/media/tasks",
            json!({
                "kind": "upload", "person": "@someone-else:example.com", "device": "",
                "target": {"kind": "upload", "value": []},
                "options": {"wifi_only": false, "charging_only": false, "max_bitrate": null,
                            "albums": [], "force_now": false},
            }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(failure["code"], "permission_denied");
}

#[tokio::test]
async fn a_queue_change_from_an_old_revision_gets_409_and_the_current_queue() {
    let f = fixture().await;
    let item = json!({
        "id": "t1", "kind": "audio", "title": "夜曲", "artist": null, "album": null,
        "artwork_url": null, "duration_ms": null, "series": null, "index": null,
        "played": false, "starred": false,
    });
    let enqueue =
        json!({"revision": 0, "op": {"op": "enqueue", "value": {"at": null, "item": item}}});
    let (status, first) = f
        .json(Method::POST, "/_wonton/api/media/queue", enqueue.clone())
        .await;
    assert_eq!(
        (status, first["revision"].as_u64()),
        (StatusCode::OK, Some(1))
    );
    let (status, current) = f
        .json(Method::POST, "/_wonton/api/media/queue", enqueue)
        .await;
    assert_eq!((status, current), (StatusCode::CONFLICT, first.clone()));
    let (_, read) = f
        .json(Method::GET, "/_wonton/api/media/queue", Value::Null)
        .await;
    assert_eq!(read, first);
}

#[tokio::test]
async fn a_queued_item_streams_from_its_own_plan_and_anything_else_is_gone() {
    let f = fixture().await;
    let (id, _) = f.start_session().await;
    let next = json!({
        "id": "v2", "kind": "video", "title": "下一集", "artist": null, "album": null,
        "artwork_url": null, "duration_ms": 10000, "series": null, "index": null,
        "played": false, "starred": false,
    });
    let enqueue =
        json!({"revision": 0, "op": {"op": "enqueue", "value": {"at": null, "item": next}}});
    let (status, _) = f
        .json(Method::POST, "/_wonton/api/media/queue", enqueue)
        .await;
    assert_eq!(status, StatusCode::OK);
    let plans = f.hits("POST /playback");
    for _ in 0..2 {
        let path = session_item_stream_path(id, "v2");
        let (status, _, body) = f
            .send(
                Method::GET,
                &path,
                &[("range", "bytes=8-".into())],
                Body::empty(),
            )
            .await;
        assert_eq!(
            (status, body.as_slice()),
            (StatusCode::PARTIAL_CONTENT, &b"89"[..])
        );
    }
    // 播放器一段段取，计划只要一次。
    assert_eq!(f.hits("POST /playback"), plans + 1);
    let (_, _, body) = f
        .send(
            Method::GET,
            &session_item_stream_path(id, "v9"),
            &[],
            Body::empty(),
        )
        .await;
    let failure: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(failure["code"], "media_gone");
}
