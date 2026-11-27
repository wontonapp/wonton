//! 媒体接口上任务和会话走完整流程的行为测试：下载由设备驱动、上传过期原地重开、点名来源先核、
//! 队列推给提供方、队列跨来源时按条目的来源取计划。夹具和假提供方在 [`super::media_tests`]。

use axum::Router;
use axum::body::Body;
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use hyper::Method;
use serde_json::{Value, json};
use wonton_protocol::media::api::{
    session_item_stream_path, session_stream_path, task_upload_path,
};
use wonton_protocol::media::{MediaKind, MediaOperation};

use super::media_tests::{Fixture, fixture, fixture_with};
use crate::media::provider::Candidate;
use crate::media::test_support::{candidate, caps};

const MUSIC: &str = "app.example.music";

/// 第二家：能上传、能放音乐；计划里的流在 `/tune/…`。
fn music() -> (Candidate, Router) {
    let router = caps(&["upload", "playback"], &["audio", "photo"])
        .route(
            "/uploads",
            post(|| async { axum::Json(json!({"upload_id": "m1", "upload_offset": 0})) }),
        )
        .route(
            "/playback",
            post(|| async {
                axum::Json(json!({
                    "session": "music-1",
                    "stream": {"direct": "http://upstream.invalid/tune/t1.mp3"},
                    "origin_stream": null,
                    "segments": [], "audio": [], "subtitles": [],
                    "seekable": true, "duration_ms": 1_000, "expires_at": 0,
                }))
            }),
        )
        .route("/tune/{*rest}", get(|| async { "ID3" }))
        .route(
            "/playback/{session}/queue",
            put(|| async { StatusCode::NO_CONTENT }),
        );
    let candidate = candidate(
        MUSIC,
        &[MediaOperation::Upload, MediaOperation::Playback],
        &[MediaKind::Audio, MediaKind::Photo],
    );
    (candidate, router)
}

fn item(id: &str, kind: &str, source: Option<&str>) -> Value {
    json!({
        "id": id, "kind": kind, "title": id, "artist": null, "album": null, "artwork_url": null,
        "duration_ms": null, "series": null, "index": null, "played": false, "starred": false,
        "source": source,
    })
}

fn options(source: Option<&str>) -> Value {
    json!({"wifi_only": false, "charging_only": false, "max_bitrate": null, "albums": [],
           "force_now": false, "source": source})
}

async fn create(f: &Fixture, body: Value) -> (StatusCode, Value) {
    f.json(Method::POST, "/_wonton/api/media/tasks", body).await
}

async fn act(f: &Fixture, task: &str, action: &str, body: Value) -> (StatusCode, Value) {
    let path = format!("/_wonton/api/media/tasks/{task}/{action}");
    f.json(Method::POST, &path, body).await
}

fn download() -> Value {
    json!({"kind": "download", "person": "", "device": "",
           "target": {"kind": "download", "value": item("v1", "video", None)},
           "options": options(None)})
}

fn upload(size: u64, source: Option<&str>) -> Value {
    json!({"kind": "upload", "person": "", "device": "",
           "target": {"kind": "upload", "value": [{
               "item_id": "i1", "path": "DCIM/1.jpg", "size": size, "checksum": null,
               "taken_at": null, "state": {"state": "queued"}, "upload_offset": 0,
           }]},
           "options": options(source)})
}

#[tokio::test]
async fn the_device_drives_a_download_from_queued_to_done() {
    let f = fixture().await;
    let (_, task) = create(&f, download()).await;
    let id = task["id"].as_str().unwrap();
    // 还没开始取字节就能停、能继续。
    let (status, paused) = act(&f, id, "pause", json!({"reason": "user"})).await;
    assert_eq!(
        (status, paused["state"]["paused"].as_str()),
        (StatusCode::OK, Some("user"))
    );
    let (_, resumed) = act(&f, id, "resume", Value::Null).await;
    assert_eq!(resumed["state"], "queued");
    let (_, again) = act(&f, id, "resume", Value::Null).await;
    assert_eq!(again["state"], "queued");
    // 来取字节就是开始下了；报进度、写完接着走。
    let path = format!("/_wonton/api/media/tasks/{id}/items/v1/content");
    let (status, _, _) = f.send(Method::GET, &path, &[], Body::empty()).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    let (_, running) = act(&f, id, "progress", json!({"done": 4, "total": 10})).await;
    assert_eq!(running["state"], "running");
    let (status, done) = act(&f, id, "complete", Value::Null).await;
    assert_eq!(
        (status, done["state"].as_str()),
        (StatusCode::OK, Some("done"))
    );
    // 手机上本来就有、一个字节没取就写完的，也直接落 `Done`。
    let (_, other) = create(&f, download()).await;
    let (status, done) = act(&f, other["id"].as_str().unwrap(), "complete", Value::Null).await;
    assert_eq!(
        (status, done["state"].as_str()),
        (StatusCode::OK, Some("done"))
    );
}

#[tokio::test]
async fn an_upload_the_provider_expired_mid_write_reopens_in_place() {
    let f = fixture().await;
    let task = f.upload_task("DCIM/3.jpg").await;
    let start = task_upload_path(task.parse().unwrap(), "i1");
    f.json(Method::POST, &start, Value::Null).await;
    f.patch(&task, "bytes 0-4/10", b"01234").await;
    f.uploads.lock().unwrap().forget = true;
    // 续写撞上过期：核心原地重开一个，回 409 带新偏移，任务不失败。
    let (status, offset) = f.patch(&task, "bytes 5-9/10", b"56789").await;
    assert_eq!(
        (status, offset["upload_offset"].as_u64()),
        (StatusCode::CONFLICT, Some(0))
    );
    f.uploads.lock().unwrap().forget = false;
    let (status, offset) = f.patch(&task, "bytes 0-9/10", b"0123456789").await;
    assert_eq!(
        (status, offset["upload_offset"].as_u64()),
        (StatusCode::OK, Some(10))
    );
    assert_eq!(f.uploads.lock().unwrap().started.len(), 2);
    let (_, read) = f
        .json(
            Method::GET,
            &format!("/_wonton/api/media/tasks/{task}"),
            Value::Null,
        )
        .await;
    assert_eq!(read["state"], "done");
    // 收尾过的再 POST 一次，不再找提供方重开。
    let (status, offset) = f.json(Method::POST, &start, Value::Null).await;
    assert_eq!(
        (status, offset["upload_offset"].as_u64()),
        (StatusCode::OK, Some(10))
    );
    assert_eq!(f.uploads.lock().unwrap().started.len(), 2);
}

#[tokio::test]
async fn an_empty_item_is_finished_when_it_is_opened() {
    let f = fixture().await;
    let (_, task) = create(&f, upload(0, None)).await;
    let id = task["id"].as_str().unwrap();
    let start = task_upload_path(id.parse().unwrap(), "i1");
    let (status, _) = f.json(Method::POST, &start, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(f.hits("POST /uploads/u1/finish"), 1);
    let (_, read) = f
        .json(
            Method::GET,
            &format!("/_wonton/api/media/tasks/{id}"),
            Value::Null,
        )
        .await;
    assert_eq!(read["state"], "done");
}

#[tokio::test]
async fn a_named_source_is_checked_at_creation_and_kept_on_retry() {
    let f = fixture_with(vec![music()]).await;
    // 两家都能上传：没点名、主人也没定默认，开第一项时要用户选；这是请求缺东西，不是核心坏了。
    let (_, unnamed) = create(&f, upload(10, None)).await;
    let start = task_upload_path(unnamed["id"].as_str().unwrap().parse().unwrap(), "i1");
    let (status, failure) = f.json(Method::POST, &start, Value::Null).await;
    assert_eq!(
        (status, failure["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("source_required"))
    );
    // 点名一家不存在的：建任务时就拒，不等第一片。下载的来源和条目自己写的不一样，也拒，不静默挑一个。
    let (status, failure) = create(&f, upload(10, Some("app.example.nothing"))).await;
    assert_eq!(
        (status, failure["code"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("plugin_unavailable"))
    );
    let mut mismatched = download();
    mismatched["target"]["value"]["source"] = json!(super::media_tests::PROVIDER);
    mismatched["options"]["source"] = json!(MUSIC);
    let (status, failure) = create(&f, mismatched).await;
    assert_eq!(
        (status, failure["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("protocol"))
    );
    // 点名音乐那家：上传发给它；失败后重试，新任务还是发给它。
    let (status, named) = create(&f, upload(10, Some(MUSIC))).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = named["id"].as_str().unwrap();
    let start = task_upload_path(id.parse().unwrap(), "i1");
    f.json(Method::POST, &start, Value::Null).await;
    assert_eq!(f.other_hits(0, "POST /uploads"), 1);
    act(
        &f,
        id,
        "failure",
        json!({"code": "media_gone", "message": "源文件没了"}),
    )
    .await;
    let (_, retried) = act(&f, id, "retry", Value::Null).await;
    let start = task_upload_path(retried["id"].as_str().unwrap().parse().unwrap(), "i1");
    let (status, _) = f.json(Method::POST, &start, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(f.other_hits(0, "POST /uploads"), 2);
    assert_eq!(f.hits("POST /uploads"), 0);
}

#[tokio::test]
async fn a_malformed_query_gets_the_failure_envelope() {
    let f = fixture().await;
    let (status, _, body) = f
        .send(
            Method::GET,
            "/_wonton/api/media/items/x/artwork",
            &[],
            Body::empty(),
        )
        .await;
    let failure: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        (status, failure["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("protocol"))
    );
}

async fn enqueue(f: &Fixture, item: Value) {
    let (_, queue) = f
        .json(Method::GET, "/_wonton/api/media/queue", Value::Null)
        .await;
    let change = json!({"revision": queue["revision"],
                        "op": {"op": "enqueue", "value": {"at": null, "item": item}}});
    let (status, _) = f
        .json(Method::POST, "/_wonton/api/media/queue", change)
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn queue_changes_reach_the_provider_of_the_playing_session() {
    let f = fixture().await;
    // 没在播：没有上游会话可推。
    enqueue(&f, item("v2", "video", None)).await;
    assert_eq!(f.hits("PUT /playback/"), 0);
    f.start_session().await;
    enqueue(&f, item("v3", "video", None)).await;
    assert_eq!(f.hits("PUT /playback/up-1/queue"), 1);
    // 入队的条目由核心填来源；这家放不了的种类入不了队。
    let (_, queue) = f
        .json(Method::GET, "/_wonton/api/media/queue", Value::Null)
        .await;
    assert_eq!(queue["entries"][0]["source"], super::media_tests::PROVIDER);
    let change = json!({"revision": queue["revision"],
                        "op": {"op": "enqueue", "value": {"at": null, "item": item("t9", "audio", None)}}});
    let (status, failure) = f
        .json(Method::POST, "/_wonton/api/media/queue", change)
        .await;
    assert_eq!(
        (status, failure["code"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("plugin_unavailable"))
    );
}

#[tokio::test]
async fn replacing_the_queue_fills_every_source_or_changes_nothing() {
    let f = fixture().await;
    f.start_session().await;
    let replace = |revision: &Value, items: Vec<Value>| {
        json!({"revision": revision, "op": {"op": "replace",
                                            "value": {"entries": items, "cursor": 1}}})
    };
    let (status, queue) = f
        .json(
            Method::POST,
            "/_wonton/api/media/queue",
            replace(
                &json!(0),
                vec![item("v1", "video", None), item("v2", "video", None)],
            ),
        )
        .await;
    assert_eq!(
        (status, queue["cursor"].as_u64()),
        (StatusCode::OK, Some(1))
    );
    for entry in queue["entries"].as_array().unwrap() {
        assert_eq!(entry["source"], super::media_tests::PROVIDER);
    }
    assert_eq!(f.hits("PUT /playback/up-1/queue"), 1);
    // 有一项这家放不了：整个拒，队列还是上一版。
    let (status, failure) = f
        .json(
            Method::POST,
            "/_wonton/api/media/queue",
            replace(
                &queue["revision"],
                vec![item("v3", "video", None), item("t9", "audio", None)],
            ),
        )
        .await;
    assert_eq!(
        (status, failure["code"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("plugin_unavailable"))
    );
    let (_, read) = f
        .json(Method::GET, "/_wonton/api/media/queue", Value::Null)
        .await;
    assert_eq!(read, queue);
}

#[tokio::test]
async fn a_queued_item_from_another_source_plays_from_that_source() {
    let f = fixture_with(vec![music()]).await;
    let (session, _) = f.start_session().await;
    enqueue(&f, item("v1", "video", None)).await;
    enqueue(&f, item("t1", "audio", Some(MUSIC))).await;
    // 预取：这一项的计划和字节都问它自己那家。
    let path = session_item_stream_path(session, "t1");
    let (status, _, body) = f.send(Method::GET, &path, &[], Body::empty()).await;
    assert_eq!(
        (status, body.as_slice()),
        (StatusCode::OK, b"ID3".as_slice())
    );
    assert_eq!(f.other_hits(0, "POST /playback"), 1);
    // 换到这一首：会话的计划也从那家来，之后的流、队列都发给它。
    let control = json!({"kind": "control", "session": session.to_string(), "ctl": "next"});
    let (status, next) = f
        .json(
            Method::POST,
            &format!("/_wonton/api/media/sessions/{session}/control"),
            control,
        )
        .await;
    assert_eq!(
        (status, next["item"]["id"].as_str()),
        (StatusCode::OK, Some("t1"))
    );
    assert_eq!(f.other_hits(0, "POST /playback"), 2);
    assert_eq!(f.hits("POST /playback"), 1);
    let (status, _, body) = f
        .send(
            Method::GET,
            &session_stream_path(session),
            &[],
            Body::empty(),
        )
        .await;
    assert_eq!(
        (status, body.as_slice()),
        (StatusCode::OK, b"ID3".as_slice())
    );
    assert_eq!(f.other_hits(0, "PUT /playback/music-1/queue"), 1);
}

#[tokio::test]
async fn a_next_whose_new_source_fails_keeps_playing_the_old_plan() {
    let router = caps(&["playback"], &["audio"])
        .route("/playback", post(|| async { StatusCode::BAD_GATEWAY }));
    let broken = candidate(MUSIC, &[MediaOperation::Playback], &[MediaKind::Audio]);
    let f = fixture_with(vec![(broken, router)]).await;
    let (status, session) = f
        .json(
            Method::POST,
            "/_wonton/api/media/sessions",
            json!({
                "position_ms": 0, "audio": null, "subtitle": null, "device": "", "person": "",
                "item": item("v1", "video", Some(super::media_tests::PROVIDER)),
                "profile": {"direct_play": ["video/mp4"], "max_bitrate": 8000000, "hdr": false,
                            "container": ["mp4"]},
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    let id: wonton_protocol::media::PlaybackSessionId =
        session["id"].as_str().unwrap().parse().unwrap();
    enqueue(&f, item("a1", "audio", Some(MUSIC))).await;
    enqueue(&f, item("a2", "audio", Some(MUSIC))).await;
    let control = json!({"kind": "control", "session": id.to_string(), "ctl": "next"});
    let path = format!("/_wonton/api/media/sessions/{id}/control");
    let (status, _) = f.json(Method::POST, &path, control).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    // 新那家的计划没要到：提供方和计划都还是旧的，旧计划照样能放。
    let (status, _, _) = f
        .send(Method::GET, &session_stream_path(id), &[], Body::empty())
        .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(f.hits("GET /stream/v1"), 1);
    let (_, session) = f
        .json(
            Method::GET,
            &format!("/_wonton/api/media/sessions/{id}"),
            Value::Null,
        )
        .await;
    assert_eq!(session["item"]["id"], "v1");
}
