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

use super::files_tests::Home;
use super::files_upload_tests::home;
use crate::config::FILE_HOOKS_HOST;
use crate::files::test_support::any_langs;
use crate::files::uploads::NewUpload;

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
    // 作废排在钩子后面；再排一次队，轮到时它已经做完。刚做完的钩子还在宽限期里，网关在期限上还判得出“核心自己慢”。
    drop(files.upload_turn(&id).await);
    assert!(files.upload_being_created(&id), "刚做完的钩子还在宽限期里");
    let pending = home.f.dir.path().join(format!("files/pending/{id}.json"));
    assert!(!pending.exists(), "作废了，不白占 24 小时");
}

/// tusd 等不及把钩子连接关了：钩子 handler 的 future 跟着被丢掉，登记和这个上传的队要扛过它，网关才判得出是核心自己慢；
/// 钩子做完以后作废排在写记录后面，写下的待完成记录照样清掉。
#[tokio::test]
async fn a_dropped_hook_connection_is_still_reported_as_busy_and_cleans_up() {
    let (home, fake, tusd) = home().await;
    let files = home.f.files.clone();
    let device = home.device.0.to_string();

    // 替身收到建上传请求：先把这个上传的队拿走（模拟写待完成记录卡住），把网关定的编号给测试，等测试放行才回答。
    let (id_sent, got_id) = oneshot::channel();
    let (held_tx, held_rx) = oneshot::channel();
    let (release, released) = oneshot::channel::<()>();
    *fake.on_create.lock().unwrap() = Some(Box::new(move |id| {
        id_sent.send(id.clone()).unwrap();
        let files = files.clone();
        tokio::spawn(async move {
            let turn = files.upload_turn(&id).await;
            held_tx.send(()).unwrap();
            released.await.ok();
            drop(turn);
        });
    }));
    let (fire, fired) = oneshot::channel::<()>();
    *fake.before_reply.lock().unwrap() = Some(Box::pin(async move {
        fired.await.ok();
    }));
    fake.status.store(500, Ordering::SeqCst);

    // 客户端的建上传请求，由网关转给 tusd。
    let request = home
        .request(Method::POST, "/_wonton/files/uploads/")
        .header("tus-resumable", "1.0.0")
        .header("upload-length", "5")
        .body(Body::empty())
        .unwrap();
    let app = home.app.clone();
    let client = tokio::spawn(async move { app.oneshot(request).await.unwrap() });
    let id = got_id.await.unwrap();
    held_rx.await.unwrap();

    // tusd 的 pre-create 钩子还在问核心：请求还没回答，tusd 等不及，把钩子连接关了。
    let hook_body = json!({"Type": "pre-create", "Event": {
        "Upload": {"Size": 5, "MetaData": {"filename": "a.txt"}},
        "HTTPRequest": {"Method": "POST", "Header": {
            "X-Wonton-Device": [device], "X-Wonton-Upload-Id": [id.to_string()]}}}})
    .to_string();
    let hook_request = hyper::Request::builder()
        .method(Method::POST)
        .uri("/")
        .header(hyper::header::HOST, format!("{FILE_HOOKS_HOST}:8080"))
        .extension(ConnectInfo(tusd))
        .body(Body::from(hook_body))
        .unwrap();
    let app = home.app.clone();
    let hook = tokio::spawn(async move { app.oneshot(hook_request).await.unwrap() });
    while !home.f.files.upload_being_created(&id) {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    hook.abort();
    let _ = hook.await;
    assert!(
        home.f.files.upload_being_created(&id),
        "钩子连接断了，登记也要在：网关靠它判是核心自己慢"
    );

    // 现在 tusd 才回 500。
    fire.send(()).unwrap();
    let reply = client.await.unwrap();
    assert_eq!(reply.status(), StatusCode::SERVICE_UNAVAILABLE);

    // 放走卡住的队：作废排在钩子后面，钩子做完才轮到它。等它做完、记录清掉，什么都不留。
    release.send(()).unwrap();
    drop(home.f.files.upload_turn(&id).await);
    assert_nothing_left(&home, &id).await;
}

/// 钩子已经写完了、tusd 才回 500（写完和 tusd 的期限几乎同时）：刚做完的登记也还认，不把 tusd 的裸 500 转给客户端；
/// 作废照样把写下的待完成记录清掉，不白占 24 小时。
#[tokio::test]
async fn a_hook_that_finished_just_before_tusd_gave_up_is_still_reported_as_busy() {
    let (home, fake, _) = home().await;
    let files = home.f.files.clone();
    let device = home.device.0.to_string();

    // 替身收到建上传请求：钩子已经问过核心、写完待完成记录，才让替身回 500。
    let (id_sent, got_id) = oneshot::channel();
    let (done_sent, done) = oneshot::channel();
    *fake.on_create.lock().unwrap() = Some(Box::new(move |id| {
        id_sent.send(id.clone()).unwrap();
        let files = files.clone();
        let device = device.clone();
        tokio::spawn(async move {
            let upload = NewUpload {
                id: &id,
                name: Some("a.txt"),
                mime_type: None,
                size: Some(5),
                device: &device,
            };
            done_sent
                .send(files.create_upload(upload, &any_langs()).await)
                .unwrap();
        });
    }));
    *fake.before_reply.lock().unwrap() = Some(Box::pin(async move {
        done.await.unwrap().unwrap();
    }));
    fake.status.store(500, Ordering::SeqCst);

    let request = home
        .request(Method::POST, "/_wonton/files/uploads/")
        .header("tus-resumable", "1.0.0")
        .header("upload-length", "5")
        .body(Body::empty())
        .unwrap();
    let reply = home.send(request).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), "service_unavailable");

    // 刚做完的登记让作废照做：写下的待完成记录清掉，不白占 24 小时。
    let id = got_id.await.unwrap();
    assert_nothing_left(&home, &id).await;
}

async fn assert_nothing_left(home: &Home, id: &FileId) {
    let pending = home.f.dir.path().join(format!("files/pending/{id}.json"));
    let data = home.f.uploads_dir().join(id.as_str());
    tokio::time::timeout(Duration::from_secs(10), async {
        while pending.exists() || data.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("待完成记录和数据要清掉");
}
