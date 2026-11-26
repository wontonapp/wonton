//! 建上传时 tusd 等不及核心（#647）：tusd 的 pre-create 钩子超过期限就回 500，正文和它自己出错时一样。

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use hyper::{Method, StatusCode};
use serde_json::json;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tower::ServiceExt;
use wonton_protocol::FileId;

use super::files_upload_tests::home;
use crate::config::FILE_HOOKS_HOST;

/// 钩子还在做（这里拿着这个上传的队卡住它，实测是磁盘忙），tusd 就回了 500：说成忙、稍后再试，不转 tusd 的裸 500，也不等卡住的
/// 钩子；钩子做完以后编号照样作废，不留待完成记录。
#[tokio::test]
async fn a_hook_tusd_stopped_waiting_for_is_reported_as_busy() {
    let (home, fake, tusd) = home().await;
    let files = home.f.files.clone();
    let (app, device) = (home.app.clone(), home.device.0.to_string());
    let created: Arc<Mutex<Option<FileId>>> = Arc::default();
    let hook: Arc<Mutex<Option<JoinHandle<()>>>> = Arc::default();
    let (release, released) = oneshot::channel::<()>();

    let seen = created.clone();
    *fake.on_create.lock().unwrap() = Some(Box::new(move |id| {
        *seen.lock().unwrap() = Some(id);
    }));
    fake.status.store(500, Ordering::SeqCst);
    let (id_of, hook_slot, stuck) = (created.clone(), hook.clone(), files.clone());
    *fake.before_reply.lock().unwrap() = Some(Box::pin(async move {
        let id = id_of.lock().unwrap().clone().unwrap();
        let (held, holding) = oneshot::channel();
        let (queue, queued) = (stuck.clone(), id.clone());
        tokio::spawn(async move {
            let _turn = queue.upload_turn(&queued).await;
            held.send(()).unwrap();
            released.await.ok();
        });
        holding.await.unwrap();
        let body = json!({"Type": "pre-create", "Event": {
            "Upload": {"Size": 5, "MetaData": {"filename": "a.txt"}},
            "HTTPRequest": {"Method": "POST", "Header": {
                "X-Wonton-Device": [device], "X-Wonton-Upload-Id": [id.to_string()]}}}});
        let request = hyper::Request::builder()
            .method(Method::POST)
            .uri("/")
            .header(hyper::header::HOST, format!("{FILE_HOOKS_HOST}:8080"))
            .extension(ConnectInfo(tusd))
            .body(Body::from(body.to_string()))
            .unwrap();
        *hook_slot.lock().unwrap() = Some(tokio::spawn(async move {
            app.oneshot(request).await.unwrap();
        }));
        while !stuck.upload_being_created(&id) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }));

    let request = home
        .request(Method::POST, "/_wonton/files/uploads/")
        .header("tus-resumable", "1.0.0")
        .header("upload-length", "5")
        .body(Body::empty())
        .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(10), home.send(request))
        .await
        .expect("钩子卡着时也要先回答，不等它");
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), "service_unavailable");

    release.send(()).unwrap();
    let hook = hook.lock().unwrap().take().unwrap();
    hook.await.unwrap();
    let id = created.lock().unwrap().clone().unwrap();
    // 作废排在钩子后面；再排一次队，轮到时它已经做完。
    drop(files.upload_turn(&id).await);
    assert!(!files.upload_being_created(&id));
    let pending = home.f.dir.path().join(format!("files/pending/{id}.json"));
    assert!(!pending.exists(), "作废了，不白占 24 小时");
}
