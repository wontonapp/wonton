//! 隧道流交给嵌入方：核心在进程内服务每条流，并知道它来自哪台已配对设备。

use iroh::SecretKey;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    task::JoinHandle,
    time::timeout,
};
use wonton_tunnel::{
    Allowlist, ERROR_HEADER, PeerStream, TunnelError, TunnelListener, TunnelServer, TunnelStatus,
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
    // 像长轮询一样：嵌入方收下请求，还没回响应。
    let Held {
        server,
        device,
        phone,
        mut tcp,
        mut stream,
        _listener,
    } = held_request().await;

    assert!(server.allowlist().revoke(&device.public()));

    assert_stream_broken(&mut stream).await;
    // 手机那边只有两种结果，取决于吊销时手机有没有读到服务端交出流前回的“已接受”：读到了，本地代理
    // 正在转发，只能断开；还没读到，QUIC 关连接时丢掉没读的数据，本地代理看到“还没回响应就被拒”，
    // 回 403 说明设备被吊销。不能有嵌入方的响应，也不能是别的错误。
    let raw = read_until_closed(&mut tcp).await;
    let text = String::from_utf8_lossy(&raw);
    let revoked = text.starts_with("HTTP/1.1 403 ")
        && text.contains(&format!(
            "{ERROR_HEADER}: {}\r\n",
            TunnelError::Unauthorized.code()
        ));
    assert!(
        raw.is_empty() || revoked,
        "吊销后只能断开或回吊销的 403：{text:?}"
    );
    phone
        .wait_for(|status| *status == TunnelStatus::Disconnected(TunnelError::Unauthorized))
        .await;
}

#[tokio::test]
async fn revoking_a_device_cuts_a_response_in_progress() {
    let Held {
        server,
        device,
        phone,
        mut tcp,
        mut stream,
        _listener,
    } = held_request().await;
    // 响应已经开始往手机走：等开头到了本地连接再吊销，手机这时一定读过“已接受”，吊销只能断开连接。
    let started = b"HTTP/1.1 200 OK\r\n";
    stream.write_all(started).await.unwrap();
    let mut seen = vec![0u8; started.len()];
    timeout(WAIT, tcp.read_exact(&mut seen))
        .await
        .expect("手机应先收到响应的开头")
        .unwrap();
    assert_eq!(seen, started);

    assert!(server.allowlist().revoke(&device.public()));

    assert_stream_broken(&mut stream).await;
    let raw = read_until_closed(&mut tcp).await;
    assert!(raw.is_empty(), "吊销后不应再收到任何数据：{raw:?}");
    phone
        .wait_for(|status| *status == TunnelStatus::Disconnected(TunnelError::Unauthorized))
        .await;
}

/// 一台已配对的手机经本地代理发了一个请求，嵌入方收下、读完请求头，还没回。
struct Held {
    server: TunnelServer,
    device: SecretKey,
    phone: Phone,
    tcp: TcpStream,
    stream: PeerStream,
    _listener: TunnelListener,
}

async fn held_request() -> Held {
    let device = SecretKey::generate();
    let (server, mut listener) = embedded_home(Allowlist::new([device.public()])).await;
    let phone = Phone::start(device.clone(), server.endpoint().addr(), Timeouts::Default).await;
    let mut tcp = TcpStream::connect(phone.proxy.local_addr()).await.unwrap();
    tcp.write_all(b"GET /sync HTTP/1.1\r\nhost: home\r\n\r\n")
        .await
        .unwrap();
    let mut stream = timeout(WAIT, listener.accept()).await.unwrap().unwrap();
    assert_eq!(stream.remote_id(), device.public());
    read_head(&mut stream).await;
    Held {
        server,
        device,
        phone,
        tcp,
        stream,
        _listener: listener,
    }
}

async fn assert_stream_broken(stream: &mut PeerStream) {
    let mut byte = [0u8; 1];
    let read = timeout(WAIT, stream.read(&mut byte))
        .await
        .expect("吊销后嵌入方应立即读到错误");
    assert!(read.is_err(), "吊销后流应该断开，实际读到 {read:?}");
}

/// 读到本地代理关掉连接为止；连接被复位也算关掉。
async fn read_until_closed(tcp: &mut TcpStream) -> Vec<u8> {
    let mut raw = Vec::new();
    timeout(WAIT, tcp.read_to_end(&mut raw))
        .await
        .expect("等待本地代理断开连接超时")
        .ok();
    raw
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
