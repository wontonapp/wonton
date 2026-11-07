//! 复用的 keep-alive 连接被上游关掉（stale）时的转发行为：幂等、空请求体的请求重发一次；
//! 有请求体的、没有请求体的非幂等请求、新连接上的失败都不重发。

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::Body;
use http_body_util::BodyExt;
use hyper::{Request, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use super::{Endpoint, Forwarded, HttpClient, Target, forward_with_deadline};
use crate::plugin::languages::Languages;

const WAIT: Duration = Duration::from_secs(10);
const LIMIT: Duration = Duration::from_secs(30);

async fn forward(
    client: &HttpClient,
    endpoint: &Endpoint,
    request: Request<Body>,
) -> axum::response::Response {
    forward_with_deadline(
        client,
        Target {
            endpoint,
            name: "测试上游",
        },
        request,
        Forwarded {
            client: None,
            scheme: "http",
        },
        LIMIT,
        &Languages::from_header(None),
    )
    .await
}

/// 发一个空体 GET 并读完响应体，让连接回到池里。
async fn get(client: &HttpClient, endpoint: &Endpoint, path: &str) -> StatusCode {
    let request = Request::builder().uri(path).body(Body::empty()).unwrap();
    let response = forward(client, endpoint, request).await;
    let status = response.status();
    drop(response.into_body().collect().await);
    status
}

/// 起一个上游：每条连接交给 `handler`，`hits` 记收到的请求数，`accepts` 记建立的连接数。
async fn upstream<H, F>(handler: H) -> (Endpoint, Arc<AtomicUsize>, Arc<AtomicUsize>)
where
    H: Fn(TcpStream, Arc<AtomicUsize>) -> F + Send + 'static,
    F: Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::Tcp(listener.local_addr().unwrap().to_string());
    let hits = Arc::new(AtomicUsize::new(0));
    let accepts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    let accepting = Arc::clone(&accepts);
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            accepting.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(handler(stream, Arc::clone(&counter)));
        }
    });
    (endpoint, hits, accepts)
}

/// 读一个完整的请求（头加按 `Content-Length` 算的请求体），丢掉。
async fn read_request(stream: &mut TcpStream) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(stream.read_u8().await.unwrap());
    }
    let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
    let length: usize = text
        .split("\r\n")
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await.unwrap();
}

async fn respond(stream: &mut TcpStream) {
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\nok")
        .await
        .unwrap();
}

/// 第一条连接开着复用：第二条请求一到就关连接、不回应（上游关掉了它以为空闲的连接）；
/// 之后的连接（重发的请求）正常回应。
async fn close_on_reuse(mut stream: TcpStream, hits: Arc<AtomicUsize>) {
    loop {
        read_request(&mut stream).await;
        if hits.fetch_add(1, Ordering::SeqCst) == 1 {
            return;
        }
        respond(&mut stream).await;
    }
}

/// 读第一个请求就关连接、不回应。
async fn close_always(mut stream: TcpStream, hits: Arc<AtomicUsize>) {
    read_request(&mut stream).await;
    hits.fetch_add(1, Ordering::SeqCst);
}

/// 回 keep-alive 响应后马上关连接：池里的连接永远是 stale 的。
async fn keepalive_then_close(mut stream: TcpStream, hits: Arc<AtomicUsize>) {
    read_request(&mut stream).await;
    hits.fetch_add(1, Ordering::SeqCst);
    respond(&mut stream).await;
}

/// 一直开着连接、每条请求都回应。
async fn keepalive(mut stream: TcpStream, hits: Arc<AtomicUsize>) {
    loop {
        read_request(&mut stream).await;
        hits.fetch_add(1, Ordering::SeqCst);
        respond(&mut stream).await;
    }
}

/// 复用的连接被上游关掉：GET 换新连接重发一次，用户看不到 502。
#[tokio::test]
async fn get_on_a_reused_connection_is_replayed_once() {
    let (endpoint, hits, _) = upstream(close_on_reuse).await;
    let client = HttpClient::for_forwarding();
    assert_eq!(get(&client, &endpoint, "/one").await, StatusCode::OK);
    assert_eq!(get(&client, &endpoint, "/two").await, StatusCode::OK);
    assert_eq!(hits.load(Ordering::SeqCst), 3, "第二条请求重发了一次");
}

/// 没有请求体的 POST 也可能有副作用，不重发：上游只收到一次，回 502。
#[tokio::test]
async fn post_without_body_on_a_reused_connection_is_not_replayed() {
    let (endpoint, hits, _) = upstream(close_on_reuse).await;
    let client = HttpClient::for_forwarding();
    assert_eq!(get(&client, &endpoint, "/one").await, StatusCode::OK);
    let request = Request::builder()
        .method("POST")
        .uri("/side-effect")
        .body(Body::empty())
        .unwrap();
    let response = forward(&client, &endpoint, request).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(hits.load(Ordering::SeqCst), 2, "上游只收到一次");
}

/// 带请求体的请求重建不出请求体，不重发。
#[tokio::test]
async fn request_with_body_on_a_reused_connection_is_not_replayed() {
    let (endpoint, hits, _) = upstream(close_on_reuse).await;
    let client = HttpClient::for_forwarding();
    assert_eq!(get(&client, &endpoint, "/one").await, StatusCode::OK);
    let request = Request::builder()
        .method("POST")
        .uri("/form")
        .body(Body::from("a=1"))
        .unwrap();
    let response = forward(&client, &endpoint, request).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(hits.load(Ordering::SeqCst), 2, "上游只收到一次");
}

/// 带 `Idempotency-Key` 的 POST 算幂等，和 Go 一样重发。
#[tokio::test]
async fn post_with_idempotency_key_is_replayed() {
    let (endpoint, hits, _) = upstream(close_on_reuse).await;
    let client = HttpClient::for_forwarding();
    assert_eq!(get(&client, &endpoint, "/one").await, StatusCode::OK);
    let request = Request::builder()
        .method("POST")
        .uri("/retryable")
        .header("idempotency-key", "abc-123")
        .body(Body::empty())
        .unwrap();
    let response = forward(&client, &endpoint, request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(hits.load(Ordering::SeqCst), 3, "重发了一次");
}

/// 新连接上失败不重发：上游没道理在刚建好的连接上挂掉，重发还可能一直循环。
#[tokio::test]
async fn fresh_connection_failure_is_not_replayed() {
    let (endpoint, hits, _) = upstream(close_always).await;
    let status = get(&HttpClient::for_forwarding(), &endpoint, "/page").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(hits.load(Ordering::SeqCst), 1, "没有重发");
}

/// 上游每条连接都在回应后关掉：连发 GET 全靠连接池，不该出现 502。
#[tokio::test]
async fn get_does_not_fail_when_the_upstream_closes_every_connection() {
    const ROUNDS: usize = 50;
    let (endpoint, hits, _) = upstream(keepalive_then_close).await;
    let client = HttpClient::for_forwarding();
    for round in 0..ROUNDS {
        let status = timeout(WAIT, get(&client, &endpoint, "/page"))
            .await
            .unwrap();
        assert_eq!(status, StatusCode::OK, "第 {round} 次转发失败");
    }
    assert_eq!(
        hits.load(Ordering::SeqCst),
        ROUNDS,
        "每个 GET 只该被上游处理一次"
    );
}

/// 池里的空闲连接由我们先关：上游的 keep-alive 比这长时，复用的不会是上游要关的连接。
#[tokio::test]
async fn idle_connections_are_closed_before_the_upstream_does() {
    let (endpoint, hits, accepts) = upstream(keepalive).await;
    let client = HttpClient::for_forwarding();
    assert_eq!(get(&client, &endpoint, "/one").await, StatusCode::OK);
    tokio::time::sleep(crate::http::FORWARD_POOL_IDLE + Duration::from_millis(300)).await;
    assert_eq!(get(&client, &endpoint, "/two").await, StatusCode::OK);
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(accepts.load(Ordering::SeqCst), 2, "空闲期过后应重新建连接");
}
