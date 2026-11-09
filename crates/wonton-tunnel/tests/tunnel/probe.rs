//! 限时确认（probe / revalidate）什么时候算通、什么时候判死：判死只看对端在窗口里有没有确认过我们发去的包。
//!
//! 机器忙的时候，确认自己那一趟往返会被排到时限后面，连接却一直在给别的流送数据。这时换连接，正在转发的
//! 请求就会跟着断，代理回 502。反向的要求不变：对端一条确认都没有（真正的断链），仍然在时限内判死、换新连接。

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, SecretKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use wonton_tunnel::wire::{ALPN, Request, Status};
use wonton_tunnel::{LocalProxy, ProxyAccess, TunnelClient, TunnelStatus};

use crate::support::{Timeouts, http_get, loopback_endpoint};

/// 一次确认的时限：快直连默认就是 1.5 秒。
const WINDOW: Duration = Duration::from_millis(1500);
/// ping 要过这么久才回，比 [`WINDOW`] 长：每次确认都会到点。
const SLOW_PING: Duration = Duration::from_secs(3);
/// 正文分块：每块字节数、块数、块间隔。总时长（3 秒）比 [`WINDOW`] 长，确认到点时正文还在走。
const CHUNK: usize = 64;
const CHUNKS: usize = 30;
const GAP: Duration = Duration::from_millis(100);

/// 回 ping 之前先睡 [`SLOW_PING`] 的服务端，`Forward` 转给慢上游。别的请求类型这一组用不到。
#[derive(Debug)]
struct SlowPing {
    upstream: SocketAddr,
}

impl ProtocolHandler for SlowPing {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        while let Ok((send, recv)) = conn.accept_bi().await {
            tokio::spawn(serve(send, recv, self.upstream));
        }
        Ok(())
    }
}

async fn serve(mut send: SendStream, mut recv: RecvStream, upstream: SocketAddr) {
    let mut kind = [0u8; 1];
    if recv.read_exact(&mut kind).await.is_err() {
        return;
    }
    match Request::from_byte(kind[0]) {
        Some(Request::Ping) => {
            tokio::time::sleep(SLOW_PING).await;
            reply(&mut send, Status::Ok).await;
        }
        Some(Request::Forward) => forward(send, recv, upstream).await,
        _ => reply(&mut send, Status::UnsupportedRequest).await,
    }
}

async fn reply(send: &mut SendStream, status: Status) {
    if send.write_all(&[status as u8]).await.is_ok() {
        send.finish().ok();
    }
}

/// 原样转给上游，两个方向各自半关闭。
async fn forward(mut send: SendStream, mut recv: RecvStream, upstream: SocketAddr) {
    let Ok(mut tcp) = TcpStream::connect(upstream).await else {
        return reply(&mut send, Status::UpstreamUnavailable).await;
    };
    if send.write_all(&[Status::Ok as u8]).await.is_err() {
        return;
    }
    let (mut tcp_read, mut tcp_write) = tcp.split();
    let upload = async {
        while let Ok(Some(chunk)) = recv.read_chunk(8192).await {
            if tcp_write.write_all(&chunk).await.is_err() {
                break;
            }
        }
        tcp_write.shutdown().await.ok();
    };
    let download = async {
        let mut buf = vec![0u8; 8192];
        loop {
            match tcp_read.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if send.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            }
        }
        send.finish().ok();
    };
    tokio::join!(upload, download);
}

/// 慢慢回正文的上游：`/slow` 先回头，再每隔 [`GAP`] 发一块 `a`，共 [`CHUNKS`] 块；别的路径立刻回。
struct SlowBody {
    addr: SocketAddr,
    task: JoinHandle<()>,
}

impl SlowBody {
    async fn spawn() -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                tokio::spawn(respond(tcp));
            }
        });
        Self { addr, task }
    }
}

impl Drop for SlowBody {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn respond(mut tcp: TcpStream) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if tcp.read(&mut byte).await.unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let path = head.split_whitespace().nth(1).unwrap_or("/");
    let slow = path.starts_with("/slow");
    let len = if slow { CHUNK * CHUNKS } else { path.len() };
    let _ = tcp
        .write_all(
            format!("HTTP/1.1 200 OK\r\ncontent-length: {len}\r\nconnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await;
    let block = if slow {
        vec![b'a'; CHUNK]
    } else {
        path.as_bytes().to_vec()
    };
    let blocks = if slow { CHUNKS } else { 1 };
    for i in 0..blocks {
        if slow && i > 0 {
            tokio::time::sleep(GAP).await;
        }
        if tcp.write_all(&block).await.is_err() {
            return;
        }
    }
    tcp.shutdown().await.ok();
}

/// 测试里的手机、服务端和上游放在一起，活到测试结束；服务端对连接数的计数用来断言有没有换连接。
struct Rig {
    client: TunnelClient,
    proxy: LocalProxy,
    home: Router,
    _phone: Endpoint,
    _upstream: SlowBody,
}

impl Rig {
    async fn start() -> Self {
        let upstream = SlowBody::spawn().await;
        let endpoint = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
        let home = Router::builder(endpoint)
            .accept(
                ALPN,
                SlowPing {
                    upstream: upstream.addr,
                },
            )
            .spawn();
        let phone = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
        let client = TunnelClient::new(
            phone.clone(),
            EndpointAddr::new(home.endpoint().id())
                .with_ip_addr(home.endpoint().bound_sockets()[0]),
        );
        let proxy = LocalProxy::bind(
            client.clone(),
            (Ipv4Addr::LOCALHOST, 0).into(),
            ProxyAccess::Open,
        )
        .await
        .unwrap();
        Self {
            client,
            proxy,
            home,
            _phone: phone,
            _upstream: upstream,
        }
    }

    fn opened(&self) -> u64 {
        self.home.endpoint().metrics().socket.num_conns_opened.get()
    }
}

/// 一条请求正在慢慢收正文时做确认：ping 比时限慢，可连接上有数据在走。不能换连接，请求要拿到全部正文。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_confirmation_that_gets_no_answer_keeps_the_connection_while_data_flows() {
    let rig = Rig::start().await;
    assert_eq!(http_get(rig.proxy.local_addr(), "/warm").await.status, 200);

    let reading = http_get(rig.proxy.local_addr(), "/slow");
    let confirming = async {
        tokio::time::sleep(WINDOW / 5).await;
        rig.client.revalidate().await
    };
    let (response, confirmed) = tokio::join!(reading, confirming);

    assert_eq!(response.status, 200);
    assert_eq!(response.body.len(), CHUNK * CHUNKS, "正文被截断了");
    assert!(
        response.body.bytes().all(|b| b == b'a'),
        "{:?}",
        &response.body[..response.body.len().min(80)]
    );
    confirmed.unwrap();
    assert_eq!(rig.client.status(), TunnelStatus::Connected);
    assert_eq!(rig.opened(), 1, "连接上有数据在走，不该换连接");
}
