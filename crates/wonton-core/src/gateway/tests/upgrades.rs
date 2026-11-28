//! WebSocket 式升级连接照样代理。

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::*;

#[tokio::test]
async fn websocket_style_upgrades_are_proxied() {
    let h = Harness::start(None).await;
    let app = h.app_session().await;
    let mut stream = TcpStream::connect(h.addr).await.unwrap();
    let request = format!(
        "GET /ws HTTP/1.1\r\nHost: {PLUGIN}:{}\r\nCookie: {app}\r\nConnection: Upgrade\r\nUpgrade: echo\r\n\r\n",
        h.addr.port()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(stream.read_u8().await.unwrap());
    }
    let head = String::from_utf8(head).unwrap();
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    stream.write_all(b"ping").await.unwrap();
    let mut echoed = [0u8; 4];
    stream.read_exact(&mut echoed).await.unwrap();
    assert_eq!(&echoed, b"ping");
}
