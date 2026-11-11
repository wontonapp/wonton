//! 假核心和隧道之间搬字节的小工具。

use std::net::SocketAddr;

use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use wonton_tunnel::TunnelListener;

/// 按 `content-length` 读请求体；不认识的头部长度当作没有体。
pub(crate) async fn read_body(tcp: &mut TcpStream, head: &str) -> Option<Vec<u8>> {
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    tcp.read_exact(&mut body).await.ok()?;
    Some(body)
}

/// 把隧道交给嵌入方的流接到假核心的 TCP 上，拆开后连接各自结束。
pub(crate) fn relay(mut listener: TunnelListener, upstream: SocketAddr) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(mut stream) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(mut tcp) = TcpStream::connect(upstream).await else {
                    return;
                };
                let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
            });
        }
    })
}
