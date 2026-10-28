//! 隧道流交给嵌入方：核心在进程内服务每条流，并知道它来自哪台已配对设备。

use iroh::SecretKey;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    task::JoinHandle,
    time::timeout,
};
use wonton_tunnel::{
    Allowlist, ERROR_HEADER, TunnelError, TunnelListener, TunnelServer, TunnelStatus,
};

use crate::support::{Phone, Timeouts, WAIT, loopback_endpoint, read_head};

#[tokio::test]
async fn embedder_gets_each_stream_with_the_authenticated_device_id() {
    let alice = SecretKey::generate();
    let bob = SecretKey::generate();
    let (server, listener) = embedded_home(Allowlist::new([alice.public(), bob.public()])).await;
    let _embedder = answer_with_device_id(listener);
    let alice_phone =
        Phone::start(alice.clone(), server.endpoint().addr(), Timeouts::Default).await;
    let bob_phone = Phone::start(bob.clone(), server.endpoint().addr(), Timeouts::Default).await;

    for _ in 0..3 {
        let seen_by_alice = alice_phone.get("/whoami").await;
        assert_eq!(seen_by_alice.status, 200);
        assert_eq!(seen_by_alice.body, alice.public().to_string());
        let seen_by_bob = bob_phone.get("/whoami").await;
        assert_eq!(seen_by_bob.status, 200);
        assert_eq!(seen_by_bob.body, bob.public().to_string());
    }
}

#[tokio::test]
async fn stopped_embedder_is_reported_as_upstream_unavailable() {
    let device = SecretKey::generate();
    let (server, listener) = embedded_home(Allowlist::new([device.public()])).await;
    drop(listener);
    let phone = Phone::start(device, server.endpoint().addr(), Timeouts::Default).await;

    let response = phone.get("/").await;
    assert_eq!(response.status, 503);
    assert_eq!(response.header(ERROR_HEADER), Some("upstream-unavailable"));
    // 隧道本身是好的：服务端接受了这台设备。
    assert_eq!(phone.client.status(), TunnelStatus::Connected);
}

#[tokio::test]
async fn revoking_a_device_breaks_streams_the_embedder_is_holding() {
    let device = SecretKey::generate();
    let (server, mut listener) = embedded_home(Allowlist::new([device.public()])).await;
    let phone = Phone::start(device.clone(), server.endpoint().addr(), Timeouts::Default).await;
    // 像长轮询一样：嵌入方收下请求，还没回响应。
    let mut tcp = TcpStream::connect(phone.proxy.local_addr()).await.unwrap();
    tcp.write_all(b"GET /sync HTTP/1.1\r\nhost: home\r\n\r\n")
        .await
        .unwrap();
    let mut stream = timeout(WAIT, listener.accept()).await.unwrap().unwrap();
    assert_eq!(stream.remote_id(), device.public());
    read_head(&mut stream).await;

    assert!(server.allowlist().revoke(&device.public()));

    let mut byte = [0u8; 1];
    let read = timeout(WAIT, stream.read(&mut byte))
        .await
        .expect("吊销后嵌入方应立即读到错误");
    assert!(read.is_err(), "吊销后流应该断开，实际读到 {read:?}");
    // 手机那边：请求没有拿到响应，连接被断开，隧道状态是“未授权”。
    let mut raw = Vec::new();
    timeout(WAIT, tcp.read_to_end(&mut raw))
        .await
        .expect("等待本地代理断开连接超时")
        .ok();
    assert!(raw.is_empty(), "不应收到任何响应：{raw:?}");
    phone
        .wait_for(|status| *status == TunnelStatus::Disconnected(TunnelError::Unauthorized))
        .await;
}

async fn embedded_home(allowlist: Allowlist) -> (TunnelServer, TunnelListener) {
    let endpoint = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
    TunnelServer::spawn_embedded(endpoint, allowlist)
}

/// 测试里的嵌入方：每条流读完请求头后回 200，正文是对端设备的 endpoint ID。
fn answer_with_device_id(mut listener: TunnelListener) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(mut stream) = listener.accept().await {
            tokio::spawn(async move {
                read_head(&mut stream).await;
                let body = stream.remote_id().to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                stream.shutdown().await.unwrap();
            });
        }
    })
}
