//! 上传出岔子时的回答：tusd 丢了数据、卡住了、磁盘满了。

use std::sync::atomic::Ordering;

use axum::body::Body;
use futures_util::StreamExt;
use hyper::{Method, StatusCode};
use wonton_protocol::FileId;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::files_tests::Home;
use super::files_upload_tests::{FakeTusd, home};
use crate::files::test_support::any_langs;

/// 第一次回 404 之后待完成记录没了，再问就不用去问 tusd 了。
#[tokio::test]
async fn a_lost_upload_is_not_asked_about_again() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 5, b"he").await;
    fake.status.store(404, Ordering::SeqCst);
    home.send(home.tus(Method::HEAD, &id)).await;
    assert_eq!(fake.calls(), Some(Method::HEAD));
    fake.seen.lock().unwrap().take();
    home.send(home.tus(Method::HEAD, &id)).await;
    assert_eq!(fake.calls(), None);
}

/// 放到一半时 tusd 回了 404（它的数据已经搬走了）：不能当成数据没了，要等放完，照实说放好了。
#[tokio::test]
async fn a_404_from_tusd_halfway_through_placing_waits_for_it() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 5, b"hello").await;
    fake.status.store(404, Ordering::SeqCst);
    let placing = home.f.files.upload_turn(&id).await;

    let asked = home.send(home.tus(Method::HEAD, &id));
    tokio::pin!(asked);
    let waiting = tokio::time::timeout(std::time::Duration::from_millis(200), &mut asked).await;
    assert!(waiting.is_err(), "放文件时不能下结论");
    assert_eq!(fake.calls(), Some(Method::HEAD), "tusd 已经回了 404");

    home.f
        .files
        .place_for_test(&id, &any_langs())
        .await
        .unwrap();
    drop(placing);
    let reply = asked.await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.header(hyper::header::HeaderName::from_static("upload-offset")),
        "5"
    );
    let transfers = home.f.shares_dir.join("transfers");
    assert!(transfers.join("a.txt").exists());
    assert!(!transfers.join("a (1).txt").exists());
}

/// tusd 没有这个上传了：客户端要的就是它没了，算取消成功。
#[tokio::test]
async fn cancelling_what_tusd_already_lost_succeeds() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 5, b"he").await;
    fake.status.store(404, Ordering::SeqCst);
    let reply = home.send(home.tus(Method::DELETE, &id)).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert_eq!(
        reply.header(hyper::header::HeaderName::from_static("tus-resumable")),
        "1.0.0"
    );
    let pending = home.f.dir.path().join(format!("files/pending/{id}.json"));
    assert!(!pending.exists());
}

/// tusd 卡住时取消有期限：取消占着这个上传的队，不能把放文件也卡住。
#[tokio::test]
async fn cancelling_does_not_wait_forever_for_tusd() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 5, b"he").await;
    *fake.before_reply.lock().unwrap() = Some(Box::pin(std::future::pending()));
    let reply = home.send(home.tus(Method::DELETE, &id)).await;
    assert_eq!(reply.error_kind(), "service_unavailable");
    let turn = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        home.f.files.upload_turn(&id),
    )
    .await;
    assert!(turn.is_ok(), "队要放开");
}

fn post(home: &Home) -> hyper::Request<Body> {
    home.request(Method::POST, "/_wonton/files/uploads/")
        .header("tus-resumable", "1.0.0")
        .header("x-wonton-upload-id", "the-clients-choice")
        .body(Body::empty())
        .unwrap()
}

/// 替身收到创建请求时：钩子问过核心，一个 `size` 字节的上传已经写了 `written` 字节（稀疏文件，不真占盘）。
fn creates_partly(home: &Home, fake: &FakeTusd, size: u64, written: u64) -> Arc<Mutex<PathBuf>> {
    let files = home.f.files.clone();
    let uploads = home.f.uploads_dir();
    let device = home.device.0.to_string();
    let data = Arc::new(Mutex::new(PathBuf::new()));
    let seen = data.clone();
    *fake.on_create.lock().unwrap() = Some(Box::new(move |id| {
        files.pending_for_test(&id, size, &device);
        let path = uploads.join(id.as_str());
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(written).unwrap();
        *seen.lock().unwrap() = path;
    }));
    data
}

/// 写了大半以后 500（比如网络超时）：声明的长度早超过剩余空间了，但磁盘放得下还差的部分，不是磁盘的事。客户端拿不到编号，没法接着传也
/// 没法取消，部分数据就地清掉。
#[tokio::test]
async fn a_failure_late_in_a_create_is_not_a_full_disk_and_cleans_up() {
    let (home, fake, _) = home().await;
    fake.status.store(500, Ordering::SeqCst);
    // 稀疏文件：长度比磁盘的剩余空间大，还差 5 字节。
    let stats = rustix::fs::statvfs(home.f.uploads_dir()).unwrap();
    let written = stats.f_bavail.saturating_mul(stats.f_frsize.max(1)) + (1 << 20);
    let data = creates_partly(&home, &fake, written + 5, written);

    let reply = home.send(post(&home)).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error_kind(), "", "tusd 的响应原样转");
    let data = data.lock().unwrap().clone();
    assert!(!data.exists(), "客户端拿不到编号，部分数据不留着");
    assert_ne!(
        fake.header("x-wonton-upload-id").as_deref(),
        Some("the-clients-choice"),
        "编号是网关定的"
    );
}

/// 回了 `Location`，而且确实放不下：报 `storage_full`。错误响应里没有 `Location`，客户端同样拿不到编号。
#[tokio::test]
async fn a_full_disk_while_creating_is_told_apart_and_cleans_up() {
    let (home, fake, _) = home().await;
    fake.status.store(500, Ordering::SeqCst);
    let data = creates_partly(&home, &fake, u64::MAX / 2, 2);
    // 这个替身回的编号和网关定的不是一个；判断只看响应里有没有 `Location`，用的总是网关定的。
    *fake.location.lock().unwrap() = Some(format!("/files/{}", crate::files::new_upload_id()));
    let reply = home.send(post(&home)).await;
    assert_eq!(reply.status, StatusCode::INSUFFICIENT_STORAGE);
    assert_eq!(reply.error_kind(), "storage_full");
    assert!(!data.lock().unwrap().exists());

    // 没回 `Location` 也一样。
    let data = creates_partly(&home, &fake, u64::MAX / 2, 2);
    *fake.location.lock().unwrap() = None;
    let reply = home.send(post(&home)).await;
    assert_eq!(reply.status, StatusCode::INSUFFICIENT_STORAGE);
    assert_eq!(reply.error_kind(), "storage_full");
    assert!(!data.lock().unwrap().exists());
}

/// 回了 `Location`，磁盘也放得下：客户端能接着传，留着。
#[tokio::test]
async fn a_failed_create_with_an_id_can_still_be_resumed() {
    let (home, fake, _) = home().await;
    fake.status.store(500, Ordering::SeqCst);
    let data = creates_partly(&home, &fake, 5, 2);
    *fake.location.lock().unwrap() = Some(format!("/files/{}", crate::files::new_upload_id()));
    let reply = home.send(post(&home)).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(data.lock().unwrap().exists());
}

fn age(path: &std::path::Path, secs: u64) {
    let when = std::time::SystemTime::now() - std::time::Duration::from_secs(secs);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

/// 暂停了一天多，这时接着传。tusd 写第一块之前数据的修改时间不变，只看它的话，这时碰上清理，数据、`.info`、`.lock` 都没了，客户端把整个
/// 文件传完以后拿到 `file_unknown`。
#[tokio::test]
async fn resuming_after_a_long_pause_keeps_the_upload_from_the_sweep() {
    for method in [Method::HEAD, Method::PATCH] {
        let (home, _fake, tusd) = home().await;
        let id = home.pending(tusd, 5, b"he").await;
        let data = home.f.uploads_dir().join(id.as_str());
        let record = home.f.dir.path().join(format!("files/pending/{id}.json"));
        let long_ago = crate::files::uploads::STALE_SECS + 60;
        age(&data, long_ago);
        age(&record, long_ago);

        home.send(home.tus(method.clone(), &id)).await;
        home.f.files.sweep(crate::plugin::manager::now_secs()).await;
        assert!(data.exists(), "{method}");
        assert!(record.exists(), "{method}");
    }
}

/// 没人来过的才清。别的设备来问也不算。
#[tokio::test]
async fn a_paused_upload_nobody_came_back_to_is_swept() {
    let (home, _fake, tusd) = home().await;
    let id = home.pending_of("another-device", tusd, 5, b"he").await;
    let data = home.f.uploads_dir().join(id.as_str());
    let record = home.f.dir.path().join(format!("files/pending/{id}.json"));
    let long_ago = crate::files::uploads::STALE_SECS + 60;
    age(&data, long_ago);
    age(&record, long_ago);

    let reply = home.send(home.tus(Method::HEAD, &id)).await;
    assert_eq!(reply.error_kind(), "not_shared");
    home.f.files.sweep(crate::plugin::manager::now_secs()).await;
    assert!(!data.exists());
    assert!(!record.exists());
}

/// 创建请求带着字节，传到一半客户端断了：错误说是客户端，不说连不上 tusd；已经写下的数据和待完成记录不留着占盘 24 小时。
#[tokio::test]
async fn a_client_gone_while_creating_leaves_nothing_behind() {
    let (home, fake, _) = home().await;
    let created = tusd_saw_the_upload(&home, &fake);
    *fake.before_reply.lock().unwrap() = Some(Box::pin(std::future::pending()));
    // tusd 收到请求之后才断：钩子已经问过核心了。
    let (gone, id) = tokio::sync::oneshot::channel();
    let body = futures_util::stream::once(std::future::ready(Ok(axum::body::Bytes::from_static(
        b"he",
    ))))
    .chain(futures_util::stream::once(async move {
        gone.send(created.await.unwrap()).unwrap();
        Err(std::io::Error::other("断了"))
    }));
    let request = home
        .request(Method::POST, "/_wonton/files/uploads/")
        .header("tus-resumable", "1.0.0")
        .header("upload-length", "5")
        .body(Body::from_stream(body))
        .unwrap();
    let reply = tokio::time::timeout(std::time::Duration::from_secs(10), home.send(request))
        .await
        .expect("请求体断了，转发要结束");
    assert_eq!(reply.error_kind(), "invalid_input");
    let id = id.await.unwrap();
    assert_nothing_left(&home, &id).await;
}

/// handler 被丢掉（请求断了）时也一样。
#[tokio::test]
async fn a_dropped_create_request_leaves_nothing_behind() {
    let (home, fake, _) = home().await;
    let created = tusd_saw_the_upload(&home, &fake);
    *fake.before_reply.lock().unwrap() = Some(Box::pin(std::future::pending()));
    let request = home
        .request(Method::POST, "/_wonton/files/uploads/")
        .header("tus-resumable", "1.0.0")
        .header("upload-length", "5")
        .body(Body::empty())
        .unwrap();
    let app = home.app.clone();
    let sending = tokio::spawn(async move {
        use tower::ServiceExt;
        app.oneshot(request).await
    });
    let id = created.await.unwrap();
    sending.abort();
    assert_nothing_left(&home, &id).await;
}

/// 取消的请求断了：作废照样做完，做完之前这个上传的队一直占着，清理和重试不会和它并发。
#[tokio::test]
async fn a_cancel_whose_request_went_away_still_finishes_in_its_turn() {
    let (home, fake, tusd) = home().await;
    let id = home.pending(tusd, 5, b"he").await;
    let (reached, at_tusd) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    *fake.before_reply.lock().unwrap() = Some(Box::pin(async move {
        reached.send(()).unwrap();
        released.await.unwrap();
    }));
    fake.status.store(204, Ordering::SeqCst);
    let request = home.tus(Method::DELETE, &id);
    let app = home.app.clone();
    let sending = tokio::spawn(async move {
        use tower::ServiceExt;
        app.oneshot(request).await
    });
    at_tusd.await.unwrap();
    sending.abort();
    let _ = sending.await;
    let queue = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        home.f.files.upload_turn(&id),
    );
    assert!(queue.await.is_err(), "取消还没做完，队不能放开");
    release.send(()).unwrap();
    assert_nothing_left(&home, &id).await;
}

/// 客户端在 tusd 的钩子到达之前就断开了：钩子晚到时不再记下这个上传。
#[tokio::test]
async fn a_hook_later_than_the_disconnect_leaves_nothing_behind() {
    let (home, fake, _) = home().await;
    let (seen, created) = tokio::sync::oneshot::channel();
    *fake.on_create.lock().unwrap() = Some(Box::new(move |id| seen.send(id).unwrap()));
    *fake.before_reply.lock().unwrap() = Some(Box::pin(std::future::pending()));
    let request = home
        .request(Method::POST, "/_wonton/files/uploads/")
        .header("tus-resumable", "1.0.0")
        .header("upload-length", "5")
        .body(Body::empty())
        .unwrap();
    let app = home.app.clone();
    let sending = tokio::spawn(async move {
        use tower::ServiceExt;
        app.oneshot(request).await
    });
    let id = created.await.unwrap();
    sending.abort();
    let _ = sending.await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let late = home
        .f
        .files
        .create_upload(
            crate::files::uploads::NewUpload {
                id: &id,
                name: Some("a.txt"),
                mime_type: None,
                size: Some(5),
                device: "phone",
            },
            &any_langs(),
        )
        .await;
    assert_eq!(
        late.unwrap_err().kind,
        wonton_protocol::FileFailure::InvalidInput
    );
    assert_nothing_left(&home, &id).await;
}

/// 替身收到创建请求时，像钩子问过核心、写了部分数据那样。给出网关定的编号。
fn tusd_saw_the_upload(home: &Home, fake: &FakeTusd) -> tokio::sync::oneshot::Receiver<FileId> {
    let (seen, created) = tokio::sync::oneshot::channel();
    let files = home.f.files.clone();
    let uploads = home.f.uploads_dir();
    let device = home.device.0.to_string();
    *fake.on_create.lock().unwrap() = Some(Box::new(move |id| {
        files.pending_for_test(&id, 5, &device);
        std::fs::write(uploads.join(id.as_str()), b"he").unwrap();
        seen.send(id).unwrap();
    }));
    created
}

async fn assert_nothing_left(home: &Home, id: &FileId) {
    let pending = home.f.dir.path().join(format!("files/pending/{id}.json"));
    let data = home.f.uploads_dir().join(id.as_str());
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while pending.exists() || data.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("待完成记录和数据要清掉");
}
