//! 配额的行为测试（ADR-0113“配额与清理”）：放不下的任务在 `Preparing` 就失败、点了名的不建；
//! 上传入库、下载写完各记各的账；设备报离线占用、被移除时那份清零。夹具在 [`super::media_tests`]。

use axum::http::StatusCode;
use hyper::Method;
use serde_json::{Value, json};
use wonton_protocol::media::api::{offline_path, quota_path, task_upload_path};

use super::media_tests::{Fixture, PROVIDER, fixture};
use crate::media::store::DEFAULT_PROVIDER_UPLOAD_LIMIT;

fn upload(size: u64, source: Option<&str>) -> Value {
    json!({"kind": "upload", "person": "", "device": "",
           "target": {"kind": "upload", "value": [{
               "item_id": "i1", "path": "DCIM/1.jpg", "size": size, "checksum": null,
               "taken_at": null, "state": {"state": "queued"}, "upload_offset": 0,
           }]},
           "options": {"wifi_only": false, "charging_only": false, "max_bitrate": null,
                       "albums": [], "force_now": false, "source": source}})
}

fn download() -> Value {
    json!({"kind": "download", "person": "", "device": "",
           "target": {"kind": "download", "value": {
               "id": "v1", "kind": "video", "title": "v1", "artist": null, "album": null,
               "artwork_url": null, "duration_ms": null, "series": null, "index": null,
               "played": false, "starred": false, "source": null,
           }},
           "options": {"wifi_only": false, "charging_only": false, "max_bitrate": null,
                       "albums": [], "force_now": false}})
}

async fn quota(f: &Fixture) -> Value {
    let (status, view) = f.json(Method::GET, &quota_path(), Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    view
}

async fn task_count(f: &Fixture) -> usize {
    let (_, list) = f
        .json(Method::GET, "/_wonton/api/media/tasks", Value::Null)
        .await;
    list["tasks"].as_array().unwrap().len()
}

#[tokio::test]
async fn an_upload_that_does_not_fit_is_refused_before_any_byte_moves() {
    let f = fixture().await;
    let too_big = DEFAULT_PROVIDER_UPLOAD_LIMIT + 1;
    // 点了名的那家放不下：不建任务，回服务器满了。
    let (status, refused) = f
        .json(
            Method::POST,
            "/_wonton/api/media/tasks",
            upload(too_big, Some(PROVIDER)),
        )
        .await;
    assert_eq!(
        (status, refused["code"].as_str()),
        (
            StatusCode::INSUFFICIENT_STORAGE,
            Some("server_storage_full")
        )
    );
    assert_eq!(task_count(&f).await, 0);
    // 没点名的先建着，开始传、选定那家时在 `Preparing` 拒，任务落 `Failed`，提供方一个请求都没收到。
    let (status, task) = f
        .json(
            Method::POST,
            "/_wonton/api/media/tasks",
            upload(too_big, None),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = task["id"].as_str().unwrap();
    let start = task_upload_path(id.parse().unwrap(), "i1");
    let (status, refused) = f.json(Method::POST, &start, Value::Null).await;
    assert_eq!(
        (status, refused["code"].as_str()),
        (
            StatusCode::INSUFFICIENT_STORAGE,
            Some("server_storage_full")
        )
    );
    let (_, read) = f
        .json(
            Method::GET,
            &format!("/_wonton/api/media/tasks/{id}"),
            Value::Null,
        )
        .await;
    assert_eq!(read["state"]["failed"]["code"], "server_storage_full");
    assert_eq!(f.hits("POST /uploads"), 0);
    assert_eq!(quota(&f).await["provider_upload"]["used"], 0);
}

#[tokio::test]
async fn finished_uploads_and_downloads_count_on_their_own_ledgers() {
    let f = fixture().await;
    let task = f.upload_task("DCIM/1.jpg").await;
    let start = task_upload_path(task.parse().unwrap(), "i1");
    f.json(Method::POST, &start, Value::Null).await;
    f.patch(&task, "bytes 0-4/10", b"01234").await;
    // 传到一半不记，入库了才记。
    assert_eq!(quota(&f).await["provider_upload"]["used"], 0);
    let (status, _) = f.patch(&task, "bytes 5-9/10", b"56789").await;
    assert_eq!(status, StatusCode::OK);
    let view = quota(&f).await;
    assert_eq!(view["provider_upload"]["used"], 10);
    assert_eq!(
        view["provider_upload"]["limit"],
        DEFAULT_PROVIDER_UPLOAD_LIMIT
    );
    // 提供方说库里已有同一张的不占额度。
    let dup = f.upload_task("dup").await;
    let start = task_upload_path(dup.parse().unwrap(), "i1");
    f.json(Method::POST, &start, Value::Null).await;
    f.patch(&dup, "bytes 0-9/10", b"0123456789").await;
    assert_eq!(quota(&f).await["provider_upload"]["used"], 10);

    let (_, task) = f
        .json(Method::POST, "/_wonton/api/media/tasks", download())
        .await;
    let id = task["id"].as_str().unwrap();
    let act = |action: &str| format!("/_wonton/api/media/tasks/{id}/{action}");
    f.json(
        Method::POST,
        &act("progress"),
        json!({"done": 4, "total": 10}),
    )
    .await;
    assert_eq!(quota(&f).await["device_offline"]["used"], 0);
    let (status, _) = f.json(Method::POST, &act("complete"), Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let view = quota(&f).await;
    assert_eq!(view["device_offline"]["used"], 10);
    assert_eq!(view["provider_upload"]["used"], 10);
    // 手机删了副本，报一次现在一共占多少：以它为准。
    let (status, view) = f
        .json(Method::PUT, &offline_path(), json!({"bytes": 3}))
        .await;
    assert_eq!(
        (status, &view["device_offline"]["used"]),
        (StatusCode::OK, &json!(3))
    );
}

#[tokio::test]
async fn a_removed_device_gives_back_its_offline_share() {
    let f = fixture().await;
    let (status, _) = f
        .json(Method::PUT, &offline_path(), json!({"bytes": 7}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let id = f.peer.0;
    let owner = f.gateway.device_person(Some(f.peer)).unwrap();
    let person = super::matrix_user_id(&f.gateway, &owner.username);
    super::devices::revoke_device(&f.gateway, &id, false)
        .await
        .unwrap();
    assert_eq!(f.media.store.quota(&person).unwrap().device_offline.used, 0);
}
