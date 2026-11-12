//! 限时确认（probe / revalidate）什么时候算通、什么时候判死：判死只看对端在窗口里有没有确认过我们发去的包。
//!
//! 机器忙的时候，确认自己那一趟往返会被排到时限后面，连接却一直在给别的流送数据。这时换连接，正在转发的
//! 请求就会跟着断，代理回 502。反向的要求不变：对端一条确认都没有（真正的断链），仍然在时限内判死、换新连接。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler};
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
///
/// 起的任务登记在 `served` 里：`vanish` 要把连接和流一起中止，套接字才放得掉。
#[derive(Clone, Debug)]
struct SlowPing {
    upstream: SocketAddr,
    served: Arc<Mutex<Vec<JoinHandle<()>>>>,
    /// 为真时收下 `Forward` 却永远不回状态字节：模拟服务端忙到接不动，客户端的请求卡在等回执上。
    hang_forward: bool,
}

impl ProtocolHandler for SlowPing {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        while let Ok((send, recv)) = conn.accept_bi().await {
            let streaming = tokio::spawn(serve(send, recv, self.upstream, self.hang_forward));
            self.served.lock().unwrap().push(streaming);
        }
        Ok(())
    }
}

async fn serve(
    mut send: SendStream,
    mut recv: RecvStream,
    upstream: SocketAddr,
    hang_forward: bool,
) {
    let mut kind = [0u8; 1];
    if recv.read_exact(&mut kind).await.is_err() {
        return;
    }
    match Request::from_byte(kind[0]) {
        Some(Request::Ping) => {
            tokio::time::sleep(SLOW_PING).await;
            reply(&mut send, Status::Ok).await;
        }
        // 收下请求但永远不回：客户端的请求卡在等服务端回执上，和服务器过载到接不动一样。
        Some(Request::Forward) if hang_forward => std::future::pending::<()>().await,
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

/// 测试里的服务端：自己收连接，`vanish` 能把 endpoint 连同它的套接字一起不出声地丢掉。
struct Server {
    endpoint: Option<Endpoint>,
    task: JoinHandle<()>,
    connections: Arc<Mutex<Vec<JoinHandle<()>>>>,
    key: SecretKey,
    upstream: SocketAddr,
    hang_forward: bool,
}

impl Server {
    async fn start(key: SecretKey, port: u16, upstream: SocketAddr, hang_forward: bool) -> Self {
        let endpoint = loopback_endpoint(key.clone(), port, Timeouts::Default).await;
        // 自己收连接时没有 Router 帮忙登记 ALPN，得写在自己身上。
        endpoint.set_alpns(vec![ALPN.to_vec()]);
        let accepting = endpoint.clone();
        let connections = Arc::new(Mutex::new(Vec::new()));
        let spawned = connections.clone();
        let task = tokio::spawn(async move {
            let handler = SlowPing {
                upstream,
                served: spawned.clone(),
                hang_forward,
            };
            while let Some(incoming) = accepting.accept().await {
                let Ok(accepting) = incoming.accept() else {
                    continue;
                };
                let Ok(conn) = accepting.await else {
                    continue;
                };
                if conn.alpn() != ALPN {
                    continue;
                }
                let handler = handler.clone();
                let accepted = tokio::spawn(async move {
                    handler.accept(conn).await.ok();
                });
                spawned.lock().unwrap().push(accepted);
            }
        });
        Self {
            endpoint: Some(endpoint),
            connections,
            key,
            task,
            upstream,
            hang_forward,
        }
    }

    fn endpoint(&self) -> &Endpoint {
        self.endpoint.as_ref().expect("服务端还在")
    }

    fn opened(&self) -> u64 {
        self.endpoint().metrics().socket.num_conns_opened.get()
    }

    fn port(&self) -> u16 {
        self.endpoint().bound_sockets()[0].port()
    }

    /// 不出声地消失：停掉收连接的循环，再把 endpoint 丢掉，谁也不再持有它的套接字。活着的连接没有关闭帧，
    /// 和真的把服务端进程冻住一样。
    async fn disappear(&mut self) {
        self.task.abort();
        let served: Vec<_> = self.connections.lock().unwrap().drain(..).collect();
        for connection in &served {
            connection.abort();
        }
        let _ = (&mut self.task).await;
        // 每条连接的任务被中止后才会松开它持有的 `Connection`；连接句柄还活着时 endpoint 也活，套接字不放。
        for connection in served {
            let _ = connection.await;
        }
        drop(self.endpoint.take());
    }

    /// 消失后按同样的身份和端口重启：模拟服务端悄悄换了一个 endpoint。
    async fn vanish(&mut self) {
        let (port, key, upstream, hang) = (
            self.port(),
            self.key.clone(),
            self.upstream,
            self.hang_forward,
        );
        self.disappear().await;
        let restarted = Self::start(key, port, upstream, hang).await;
        self.endpoint = restarted.endpoint;
        self.task = restarted.task;
    }
}

/// 测试里的手机、服务端和上游放在一起，活到测试结束；服务端对连接数的计数用来断言有没有换连接。
struct Rig {
    client: TunnelClient,
    proxy: LocalProxy,
    server: Server,
    _phone: Endpoint,
    _upstream: SlowBody,
}

impl Rig {
    async fn start() -> Self {
        Self::start_with_hanging_forward(false).await
    }

    /// 服务端收下 Forward 却永远不回状态字节：请求卡在等服务端回执上（催促测试用）。
    async fn start_with_hanging_forward(hang_forward: bool) -> Self {
        let upstream = SlowBody::spawn().await;
        let server = Server::start(SecretKey::generate(), 0, upstream.addr, hang_forward).await;
        let phone = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
        let client = TunnelClient::new(
            phone.clone(),
            EndpointAddr::new(server.endpoint().id())
                .with_ip_addr(server.endpoint().bound_sockets()[0]),
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
            server,
            _phone: phone,
            _upstream: upstream,
        }
    }

    fn opened(&self) -> u64 {
        self.server.opened()
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

/// 服务端不出声地换了一个 endpoint（同一身份、同一端口，客户端收不到任何通知）：断掉那一刻在路上的包会落在窗口
/// 前段。只看整个窗口有没有涨，它就被当成“还活着”留下来，和空闲超时一样要等三十秒；后半段还涨才算数。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn leftovers_of_a_silently_replaced_server_do_not_keep_the_connection() {
    let mut rig = Rig::start().await;
    assert_eq!(http_get(rig.proxy.local_addr(), "/warm").await.status, 200);

    // 正文开始走以后确认一次，确认还在窗口里（ping 比窗口慢，一定会到点）时服务端消失：断掉那一刻在路上
    // 的包正好落在窗口前段。
    let reading = tokio::spawn({
        let addr = rig.proxy.local_addr();
        async move { http_get(addr, "/slow").await }
    });
    tokio::time::sleep(GAP).await;
    let revalidating = tokio::spawn({
        let client = rig.client.clone();
        async move { client.revalidate().await }
    });
    tokio::time::sleep(GAP * 2).await;
    rig.server.vanish().await;

    revalidating.await.unwrap().unwrap();
    // 换过 endpoint 之后，计数只可能是新连接建的那一条：旧 endpoint 已经不在了。
    assert_eq!(rig.opened(), 1, "旧连接已经没人应答，必须换新连接");
    assert_eq!(rig.client.status(), TunnelStatus::Connected);
    assert_eq!(http_get(rig.proxy.local_addr(), "/after").await.status, 200);
    reading.abort();
}
/// 本地代理等服务端回执等过 [`NUDGE_AFTER`] 时催一次定时检查：检查循环收到催促
/// 立刻确认换连接，不等下一个周期。催促不能太早：中继负载下往返到过 2.8 秒（ADR-0004），
/// 一卡就催会把忙线上的慢响应误当成断链。
const NUDGE_AFTER: Duration = Duration::from_secs(2);
/// 催促应该在一个定时检查周期内到达（home/check.rs 的周期是 5 秒），再晚催促就失去意义。
const NUDGE_CEILING: Duration = Duration::from_secs(5);

/// 服务端收下请求却不回状态字节时，卡在等服务端回执上的本地代理请求自己催一次检查，不用谁手动调用。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_stalled_without_an_answer_nudges_the_periodic_check() {
    let rig = Rig::start_with_hanging_forward(true).await;
    let started = std::time::Instant::now();
    let stalled = tokio::spawn({
        let addr = rig.proxy.local_addr();
        async move { http_get(addr, "/slow").await }
    });
    // 没人手动催：这条请求在等状态字节，超时后由代理催。
    tokio::time::timeout(NUDGE_CEILING, rig.client.wait_for_check_nudge())
        .await
        .expect("请求等不到服务端回执时，代理应该催一次检查");
    let waited = started.elapsed();
    stalled.abort();
    assert!(
        waited < NUDGE_CEILING,
        "催促应该早于下一个检查周期，实际 {waited:?}"
    );
    assert!(waited >= NUDGE_AFTER, "不该一卡就催，实际 {waited:?}");
}
