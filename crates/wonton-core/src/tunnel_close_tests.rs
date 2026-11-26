//! 隧道流异常收尾：网关出错或 panic 之后，客户端要读到干净的 FIN，不是复位。
//!
//! 只走本机回环，不需要 Docker：两端各起一个 iroh endpoint，客户端在这条流上直接说 HTTP/1.1。

use std::{
    io,
    net::Ipv4Addr,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::State,
    routing::{get, post},
};
use http_body_util::BodyExt;
use hyper::{
    Response,
    body::{Bytes, Frame},
};
use iroh::{
    Endpoint, SecretKey,
    endpoint::{Connection, ReadError, RecvStream, SendStream, presets},
};
use tokio::{sync::Notify, task::JoinHandle, time::timeout};
use wonton_tunnel::{PeerStream, wire::ALPN};

use super::{CLOSE_DRAIN, serve_stream};

const WAIT: Duration = Duration::from_secs(5);

#[tokio::test]
async fn a_panicking_body_ends_the_stream_with_a_clean_fin() {
    let (logs, _scope) = capture();
    let mut stream = connect(app()).await;

    stream.send.write_all(&request("/panic")).await.unwrap();
    let (bytes, end) = stream.read_all().await;

    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.starts_with("HTTP/1.1 200"),
        "200 头没到：{text:?}（结束：{end:?}）"
    );
    assert!(text.contains("first"), "panic 前那一块没到：{text:?}");
    assert!(end.is_ok(), "读到复位而不是 EOF：{end:?}；已收到 {text:?}");

    let logs = logs.text();
    assert!(logs.contains("capture-probe"), "日志没进缓冲：{logs}");
    assert_eq!(logs.matches("tunnel_stream_aborted").count(), 1, "{logs}");
    assert!(logs.contains("side=upstream"), "{logs}");
    assert!(logs.contains("is_panic=true"), "{logs}");
    assert!(logs.contains("category=panic"), "{logs}");
    stream.finish().await;
}

#[tokio::test]
async fn a_broken_response_body_ends_the_stream_with_a_clean_fin() {
    let (logs, _scope) = capture();
    let mut stream = connect(app()).await;

    stream
        .send
        .write_all(&request("/broken-body"))
        .await
        .unwrap();
    let (bytes, end) = stream.read_all().await;

    let text = String::from_utf8_lossy(&bytes);
    assert!(end.is_ok(), "读到复位而不是 EOF：{end:?}；已收到 {text:?}");
    assert!(text.contains("first"), "出错前那一块没到：{text:?}");

    let logs = logs.text();
    assert!(logs.contains("capture-probe"), "日志没进缓冲：{logs}");
    assert_eq!(logs.matches("tunnel_stream_aborted").count(), 1, "{logs}");
    assert!(logs.contains("side=upstream"), "{logs}");
    assert!(logs.contains("is_panic=false"), "{logs}");
    assert!(logs.contains("category=response_body_aborted"), "{logs}");
    stream.finish().await;
}

#[tokio::test]
async fn a_second_request_on_a_broken_keep_alive_stream_is_not_swallowed() {
    let mut stream = connect(app()).await;

    stream
        .send
        .write_all(&request("/broken-body"))
        .await
        .unwrap();
    let (_, end) = stream.read_all().await;
    assert!(end.is_ok(), "第一个响应没以 EOF 收尾：{end:?}");

    // 同一条流上再发一个请求：连接已经结束，写可能被拒，但读必须立刻给 EOF，客户端才知道要重拨。
    let _ = stream.send.write_all(&request("/b")).await;
    let (after, end) = stream.read_all().await;
    assert!(end.is_ok(), "第二个请求之后读不到 EOF：{end:?}");
    assert!(
        String::from_utf8_lossy(&after).trim().is_empty(),
        "连接已经结束却回了半个响应：{:?}",
        String::from_utf8_lossy(&after)
    );
    stream.finish().await;

    let mut redialed = connect(app()).await;
    redialed.send.write_all(&request("/b")).await.unwrap();
    let (bytes, end) = redialed.read_upto(b"second").await;
    let text = String::from_utf8_lossy(&bytes);
    assert!(end.is_ok(), "{end:?}");
    assert!(
        text.starts_with("HTTP/1.1 200"),
        "重拨后没拿到 200：{text:?}"
    );
    assert!(text.contains("second"), "{text:?}");
    redialed.finish().await;
}

/// 请求体还在途（handler 读过一帧、`Content-Length` 没发完）时出错：先给客户端 FIN，再排空读半边。
/// 对端不发完也不复位，排空得在 `CLOSE_DRAIN` 上限收尾，任务不能一直挂着。
#[tokio::test(start_paused = true)]
async fn an_in_flight_request_body_is_drained_until_the_close_bound() {
    let (logs, _scope) = capture();
    let probed = Arc::new(Notify::new());
    let mut stream = connect(app_with(probed.clone())).await;

    stream.send.write_all(&upload_head()).await.unwrap();
    stream.send.write_all(&[b'x'; 10]).await.unwrap();
    timeout(WAIT, probed.notified())
        .await
        .expect("handler 没读请求体");

    let (bytes, end) = stream.read_all().await;
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.starts_with("HTTP/1.1 200"),
        "200 头没到：{text:?}（结束：{end:?}）"
    );
    assert!(text.contains("first"), "出错前那一块没到：{text:?}");
    assert!(end.is_ok(), "读到复位而不是 EOF：{end:?}；已收到 {text:?}");

    // 请求体还差 90 字节、对端也没发 FIN：任务必须停在排空读半边上。
    tokio::time::sleep(CLOSE_DRAIN / 2).await;
    assert!(!stream.task.is_finished(), "有在途请求却没排空就结束了");
    tokio::time::sleep(CLOSE_DRAIN / 2 + Duration::from_millis(100)).await;
    assert!(stream.task.is_finished(), "过了上限还在排空");

    let logs = logs.text();
    assert!(logs.contains("没在 2s 内读完"), "{logs}");
    stream.finish().await;
}

/// 排空途中对端复位：读半边报错，排空立即收尾，不必等满上限。
#[tokio::test(start_paused = true)]
async fn a_peer_reset_ends_the_drain_early() {
    let (logs, _scope) = capture();
    let probed = Arc::new(Notify::new());
    let mut stream = connect(app_with(probed.clone())).await;

    stream.send.write_all(&upload_head()).await.unwrap();
    stream.send.write_all(&[b'x'; 10]).await.unwrap();
    timeout(WAIT, probed.notified())
        .await
        .expect("handler 没读请求体");

    let (_, end) = stream.read_all().await;
    assert!(end.is_ok(), "读到复位而不是 EOF：{end:?}");
    tokio::time::sleep(CLOSE_DRAIN / 2).await;
    assert!(!stream.task.is_finished(), "有在途请求却没排空就结束了");

    stream.send.reset(0u32.into()).unwrap();
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert!(stream.task.is_finished(), "对端复位之后还在排空");

    let logs = logs.text();
    assert!(!logs.contains("没在"), "复位了却等到上限：{logs}");
    stream.finish().await;
}

#[tokio::test]
async fn two_requests_share_one_keep_alive_stream() {
    let (logs, _scope) = capture();
    let mut stream = connect(app()).await;

    for _ in 0..2 {
        stream.send.write_all(&request("/b")).await.unwrap();
        let (bytes, end) = stream.read_upto(b"second").await;
        let text = String::from_utf8_lossy(&bytes);
        assert!(end.is_ok(), "{end:?}");
        assert!(text.starts_with("HTTP/1.1 200"), "{text:?}");
    }
    stream.send.finish().ok();
    let (_, end) = stream.read_all().await;
    assert!(end.is_ok(), "客户端收尾后服务端也没给 EOF：{end:?}");

    let logs = logs.text();
    assert!(logs.contains("隧道上的 HTTP 连接结束"), "{logs}");
    assert!(logs.contains("side=downstream"), "{logs}");
    assert!(logs.contains("category=closed"), "{logs}");
    stream.finish().await;
}

fn app() -> Router {
    app_with(Arc::new(Notify::new()))
}

fn app_with(probed: Arc<Notify>) -> Router {
    Router::new()
        .route("/b", get(|| async { "second" }))
        .route(
            "/panic",
            get(|| async { Response::new(Body::new(HalfBody::new(End::Panic))) }),
        )
        .route(
            "/broken-body",
            get(|| async { Response::new(Body::new(HalfBody::new(End::Error))) }),
        )
        .route("/upload", post(upload))
        .with_state(probed)
}

/// 读一帧请求体、通知测试（在途标记已置真），然后让响应体出错。
async fn upload(State(probed): State<Arc<Notify>>, body: Body) -> Response<Body> {
    let mut body = body;
    assert!(body.frame().await.is_some(), "请求体一帧都没有");
    probed.notify_one();
    Response::new(Body::new(HalfBody::new(End::Error)))
}

fn request(path: &str) -> Vec<u8> {
    format!("GET {path} HTTP/1.1\r\nHost: gateway\r\n\r\n").into_bytes()
}

fn upload_head() -> Vec<u8> {
    b"POST /upload HTTP/1.1\r\nHost: gateway\r\nContent-Length: 100\r\n\r\n".to_vec()
}

/// 响应体写到一半怎么结束。
#[derive(Clone, Copy)]
enum End {
    Panic,
    Error,
}

/// 第一帧给数据，第二帧先挂起再自己唤醒：hyper 在挂起的那一次 poll 里会接着 flush，头和数据
/// 真到对端，第三帧才 panic 或回错。不这样的话失败发生在同一个 poll 里，hyper 还没 flush。
struct HalfBody {
    polls: u8,
    end: End,
}

impl HalfBody {
    fn new(end: End) -> Self {
        Self { polls: 0, end }
    }
}

impl hyper::body::Body for HalfBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let this = self.get_mut();
        match this.polls {
            0 => {
                this.polls = 1;
                Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"first")))))
            }
            1 => {
                this.polls = 2;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            _ => match this.end {
                End::Panic => panic!("响应体写到一半 panic"),
                End::Error => Poll::Ready(Some(Err(io::Error::other("body broken")))),
            },
        }
    }
}

/// 一条接通的隧道流：服务端已经交给 `serve_stream`，这里拿着客户端那半边。
struct Streams {
    server: Endpoint,
    client: Endpoint,
    conn: Connection,
    /// 服务端那条连接：生产里由 `serve` 持有，这里留着免得连接随流任务一起结束。
    serving: Connection,
    task: JoinHandle<()>,
    send: SendStream,
    recv: RecvStream,
}

async fn loopback_endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap()
}

async fn connect(app: Router) -> Streams {
    let server = loopback_endpoint().await;
    let client = loopback_endpoint().await;
    let addr = server.addr();
    let serving = server.clone();
    let (keep_tx, keep_conn) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let incoming = serving.accept().await.expect("endpoint 被关了");
        let conn = incoming.await.unwrap();
        keep_tx.send(conn.clone()).ok();
        let (send, recv) = conn.accept_bi().await.unwrap();
        serve_stream(PeerStream::new(conn.remote_id(), send, recv), app).await;
    });
    let conn = timeout(WAIT, client.connect(addr, ALPN))
        .await
        .expect("连不上服务端 endpoint")
        .unwrap();
    let serving = timeout(WAIT, keep_conn)
        .await
        .expect("服务端把连接交出来")
        .unwrap();
    let (send, recv) = timeout(WAIT, conn.open_bi())
        .await
        .expect("开不出流")
        .unwrap();
    Streams {
        server,
        client,
        conn,
        serving,
        task,
        send,
        recv,
    }
}

impl Streams {
    /// 读到 EOF（`Ok`）或复位（`Err`），返回读到的字节。
    async fn read_all(&mut self) -> (Vec<u8>, Result<(), ReadError>) {
        let mut bytes = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            match timeout(WAIT, self.recv.read(&mut buf)).await {
                Ok(Ok(Some(n))) => bytes.extend_from_slice(&buf[..n]),
                Ok(Ok(None)) => return (bytes, Ok(())),
                Ok(Err(err)) => return (bytes, Err(err)),
                Err(_) => panic!(
                    "读超时，流既没结束也没复位：{:?}",
                    String::from_utf8_lossy(&bytes)
                ),
            }
        }
    }

    /// 读到 `tail` 出现为止（一个完整响应的结尾）。
    async fn read_upto(&mut self, tail: &[u8]) -> (Vec<u8>, Result<(), ReadError>) {
        let mut bytes = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            match timeout(WAIT, self.recv.read(&mut buf)).await {
                Ok(Ok(Some(n))) => bytes.extend_from_slice(&buf[..n]),
                Ok(Ok(None)) => return (bytes, Ok(())),
                Ok(Err(err)) => return (bytes, Err(err)),
                Err(_) => panic!("读超时：{:?}", String::from_utf8_lossy(&bytes)),
            }
            if bytes.ends_with(tail) {
                return (bytes, Ok(()));
            }
        }
    }

    async fn finish(self) {
        self.conn.close(0u32.into(), b"done");
        self.serving.close(0u32.into(), b"done");
        let joined = timeout(WAIT, self.task).await.expect("流任务没结束");
        assert!(joined.is_ok(), "连接任务 panic 外溢");
        self.client.close().await;
        self.server.close().await;
    }
}

/// 收进一个缓冲的日志，测试里查有没有那条 ERROR。线程本地，不抢全局订阅者。
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logs {
    type Writer = Logs;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Logs {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

fn capture() -> (Logs, tracing::subscriber::DefaultGuard) {
    let logs = Logs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    tracing::error!("capture-probe");
    (logs, guard)
}
