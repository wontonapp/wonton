//! 吊销设备切断正在进行的传输：核心收到吊销就关掉这台设备的隧道连接，所以不用在分块之间重新鉴权。真的隧道，只走回环地址。

use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::Request;
use futures_util::stream;
use http_body_util::BodyExt;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

use super::files_tests::{Home, home_with};
use super::tests::CORE;
use super::tunnel_tests::Phone;
use crate::tunnel::TunnelEntry;

/// 比隧道的流控窗口大得多：吊销时不可能都在路上了。
const LARGE: usize = 64 << 20;
const GIVE_UP: Duration = Duration::from_secs(15);

/// 只有这台手机配对的家。
async fn paired(tusd: Option<String>) -> (Home, Phone, SecretKey, TunnelEntry) {
    let home = home_with(tusd).await;
    let key = SecretKey::generate();
    home.tunnel.devices.add(key.public(), None).unwrap();
    let entry = TunnelEntry::spawn(&home.tunnel, home.app.clone());
    let phone = Phone::connect(key.clone(), &home.tunnel).await;
    (home, phone, key, entry)
}

#[tokio::test]
async fn revoking_a_device_cuts_its_download() {
    let (home, phone, key, _entry) = paired(None).await;
    let id = home.f.uploaded("big.bin", &vec![7; LARGE]);
    let request = phone
        .request(Method::GET, CORE, &format!("/_wonton/files/{id}/content"))
        .header("x-wonton-files", "1")
        .body(Body::empty())
        .unwrap();
    let response = phone.http().send(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let first = body.frame().await.unwrap().unwrap();
    let mut read = first.into_data().unwrap().len();

    home.tunnel.devices.remove(&key.public()).unwrap();

    let cut = tokio::time::timeout(GIVE_UP, async {
        while let Some(frame) = body.frame().await {
            match frame {
                Ok(frame) => read += frame.into_data().map_or(0, |data| data.len()),
                Err(_) => return true,
            }
        }
        false
    })
    .await
    .expect("吊销后下载还在继续");
    assert!(cut, "读到了正常的结尾");
    assert!(read < LARGE, "读完了 {read} 字节");
}

/// 记下收到的请求体的 tusd 替身：`first` 收到第一块，`end` 请求体结束，值是读完了没有（中断是假）。
async fn tusd_watching_the_body() -> (SocketAddr, oneshot::Receiver<()>, oneshot::Receiver<bool>) {
    let (first_tx, first) = oneshot::channel();
    let (end_tx, end) = oneshot::channel();
    let slots = std::sync::Arc::new(std::sync::Mutex::new(Some((first_tx, end_tx))));
    let app = Router::new().fallback(move |req: Request| {
        let slots = slots.clone();
        async move {
            if req.method() != Method::PATCH {
                return StatusCode::OK;
            }
            let (first_tx, end_tx) = slots.lock().unwrap().take().unwrap();
            let mut first_tx = Some(first_tx);
            let mut body = req.into_body();
            let finished = loop {
                match body.frame().await {
                    Some(Ok(_)) => {
                        if let Some(first_tx) = first_tx.take() {
                            let _ = first_tx.send(());
                        }
                    }
                    Some(Err(_)) => break false,
                    None => break true,
                }
            };
            let _ = end_tx.send(finished);
            StatusCode::NO_CONTENT
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, first, end)
}

#[tokio::test]
async fn revoking_a_device_cuts_its_upload() {
    let (tusd, first, end) = tusd_watching_the_body().await;
    let (home, phone, key, _entry) = paired(Some(tusd.to_string())).await;
    let device = key.public().to_string();
    let id = home.pending_of(&device, tusd, 1 << 20, b"").await;

    let (chunks, mut fed) = mpsc::channel::<Bytes>(1);
    let body = Body::from_stream(stream::poll_fn(move |cx| {
        fed.poll_recv(cx)
            .map(|chunk| chunk.map(Ok::<_, Infallible>))
    }));
    let request = phone
        .request(Method::PATCH, CORE, &format!("/_wonton/files/uploads/{id}"))
        .header("x-wonton-files", "1")
        .header("tus-resumable", "1.0.0")
        .header("upload-offset", "0")
        .header("content-type", "application/offset+octet-stream")
        .body(body)
        .unwrap();
    let http = phone.http();
    let patch = tokio::spawn(async move { http.send(request).await });
    chunks.send(Bytes::from(vec![7; 64 << 10])).await.unwrap();
    tokio::time::timeout(GIVE_UP, first)
        .await
        .expect("第一块没到 tusd")
        .unwrap();

    home.tunnel.devices.remove(&key.public()).unwrap();

    let finished = tokio::time::timeout(GIVE_UP, end)
        .await
        .expect("吊销后 tusd 的请求体还开着")
        .unwrap();
    assert!(!finished, "请求体是被切断的，不是传完的");
    patch.abort();
}
