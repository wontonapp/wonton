use std::{convert::Infallible, time::Duration};

use axum::body::{Body, Bytes};
use http_body_util::BodyExt;
use hyper::{Method, Request, StatusCode, header};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UnixListener},
    sync::oneshot,
    time::timeout,
};

use super::{Endpoint, Forwarded, HttpClient, Target, forward_with_limit};
use crate::plugin::languages::Languages;

const LIMIT: Duration = Duration::from_millis(300);
const WAIT: Duration = Duration::from_secs(10);

async fn head(io: &mut (impl AsyncRead + Unpin)) -> Vec<u8> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        bytes.push(io.read_u8().await.unwrap());
    }
    bytes
}

async fn forward(endpoint: &Endpoint, request: Request<Body>) -> axum::response::Response {
    forward_with_limit(
        &HttpClient::new(),
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
    .unwrap()
}

/// 网页入口（`proxy::forward_with_deadline`）：失败由转发层回 HTML 错误页。
async fn web(endpoint: &Endpoint, request: Request<Body>) -> axum::response::Response {
    super::forward_with_deadline(
        &HttpClient::new(),
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

/// API 入口（`proxy::forward_or`）：失败原样交给调用方。
async fn api(
    endpoint: &Endpoint,
    request: Request<Body>,
) -> Result<axum::response::Response, super::ForwardError> {
    forward_with_limit(
        &HttpClient::new(),
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

fn get(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn silent_tcp_upstream_is_a_header_timeout_not_a_transport_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::Tcp(listener.local_addr().unwrap().to_string());
    let serving = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        head(&mut stream).await;
        let mut bytes = Vec::new();
        timeout(WAIT, stream.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert!(bytes.is_empty(), "空体请求不应被重放");
    });
    let response = timeout(WAIT, web(&endpoint, get("/silent"))).await.unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(response.headers()["x-wonton-error"], "upstream_timeout");
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    serving.await.unwrap();
}

#[tokio::test]
async fn forward_or_hands_the_typed_timeout_to_its_caller() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::Tcp(listener.local_addr().unwrap().to_string());
    let serving = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        head(&mut stream).await;
        // 连接一直开着，期限才能触发；提前关掉会被 hyper 记成连接失败。
        tokio::time::sleep(LIMIT * 3).await;
    });
    let error = timeout(WAIT, api(&endpoint, get("/silent"))).await.unwrap();
    assert!(
        matches!(error, Err(super::ForwardError::HeaderTimeout)),
        "{error:?}"
    );
    serving.await.unwrap();
}

#[tokio::test]
async fn silent_unix_upstream_uses_the_same_header_deadline() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("upstream.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let serving = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        head(&mut stream).await;
        let mut bytes = Vec::new();
        timeout(WAIT, stream.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
    });
    let response = timeout(WAIT, web(&Endpoint::unix(socket), get("/silent")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(response.headers()["x-wonton-error"], "upstream_timeout");
    serving.await.unwrap();
}

#[tokio::test]
async fn matrix_long_polls_and_sliding_sync_are_not_timed_out() {
    for path in [
        "/_matrix/client/v3/sync?timeout=300000",
        "/_matrix/client/unstable/org.matrix.msc3575/sync",
        "/_matrix/media/v3/download/x/y",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = Endpoint::Tcp(listener.local_addr().unwrap().to_string());
        let serving = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            head(&mut stream).await;
            tokio::time::sleep(LIMIT * 3).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}")
                .await
                .unwrap();
        });
        let response = timeout(WAIT, forward(&endpoint, get(path))).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            "{}"
        );
        serving.await.unwrap();
    }
}

#[tokio::test]
async fn streaming_response_continues_after_the_header_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::Tcp(listener.local_addr().unwrap().to_string());
    let serving = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        head(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 8\r\nconnection: close\r\n\r\nfirst")
            .await
            .unwrap();
        tokio::time::sleep(LIMIT * 3).await;
        stream.write_all(b"end").await.unwrap();
    });
    let response = timeout(WAIT, forward(&endpoint, get("/stream")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = timeout(WAIT, response.into_body().collect())
        .await
        .unwrap()
        .unwrap()
        .to_bytes();
    assert_eq!(bytes, "firstend");
    serving.await.unwrap();
}

#[tokio::test]
async fn upload_time_is_excluded_for_fixed_length_and_chunked_bodies() {
    for fixed in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = Endpoint::Tcp(listener.local_addr().unwrap().to_string());
        let (started, start) = oneshot::channel();
        let serving = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let started = std::sync::Mutex::new(Some(started));
            let service = hyper::service::service_fn(move |req: Request<hyper::body::Incoming>| {
                let started = started.lock().unwrap().take().unwrap();
                async move {
                    started.send(()).unwrap();
                    let bytes = req.into_body().collect().await.unwrap().to_bytes();
                    assert_eq!(bytes, "slow upload");
                    Ok::<_, Infallible>(hyper::Response::new(Body::from("accepted")))
                }
            });
            hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .await
                .unwrap();
        });
        let body = Body::from_stream(futures_util::stream::once(async move {
            start.await.unwrap();
            tokio::time::sleep(LIMIT * 3).await;
            Ok::<_, Infallible>(Bytes::from_static(b"slow upload"))
        }));
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/upload")
            .header(header::CONNECTION, "close");
        if fixed {
            request = request.header(header::CONTENT_LENGTH, 11);
        }
        let response = timeout(WAIT, forward(&endpoint, request.body(body).unwrap()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "fixed length: {fixed}");
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            "accepted"
        );
        serving.await.unwrap();
    }
}

#[tokio::test]
async fn pending_socket_writes_are_still_upload_not_header_wait() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::Tcp(listener.local_addr().unwrap().to_string());
    let serving = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        head(&mut stream).await;
        tokio::time::sleep(LIMIT * 3).await;
        let mut remaining = 16 * 1024 * 1024;
        let mut buffer = [0; 64 * 1024];
        while remaining > 0 {
            let read = stream.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0, "上传不能被响应头期限打断");
            remaining -= read;
        }
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n")
            .await
            .unwrap();
    });
    let request = Request::builder()
        .method(Method::POST)
        .uri("/large")
        .header(header::CONTENT_LENGTH, 16 * 1024 * 1024)
        .body(Body::from(vec![b'x'; 16 * 1024 * 1024]))
        .unwrap();
    let response = timeout(WAIT, forward(&endpoint, request)).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    serving.await.unwrap();
}

#[tokio::test]
async fn upgrade_handshake_is_not_a_header_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::Tcp(listener.local_addr().unwrap().to_string());
    let serving = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        head(&mut stream).await;
        tokio::time::sleep(LIMIT * 3).await;
        stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nconnection: upgrade\r\nupgrade: websocket\r\n\r\n").await.unwrap();
    });
    let request = Request::builder()
        .uri("/socket")
        .header(header::CONNECTION, "upgrade")
        .header(header::UPGRADE, "websocket")
        .body(Body::empty())
        .unwrap();
    let response = timeout(WAIT, forward(&endpoint, request)).await.unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    serving.await.unwrap();
}

#[tokio::test]
async fn the_deadline_starts_after_a_fixed_length_stream_without_polling_eof() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::Tcp(listener.local_addr().unwrap().to_string());
    let serving = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        head(&mut stream).await;
        let mut body = [0; 4];
        stream.read_exact(&mut body).await.unwrap();
        assert_eq!(&body, b"data");
        let mut rest = Vec::new();
        timeout(WAIT, stream.read_to_end(&mut rest))
            .await
            .unwrap()
            .unwrap();
        assert!(rest.is_empty(), "请求不能被自动重放");
    });
    // hyper 按 Content-Length 发完即可，不保证再 poll 一次 None。
    let stream = futures_util::stream::iter([Ok::<_, Infallible>(Bytes::from_static(b"data"))]);
    let request = Request::builder()
        .method(Method::POST)
        .uri("/upload")
        .header(header::CONTENT_LENGTH, 4)
        .body(Body::from_stream(stream))
        .unwrap();
    let error = timeout(WAIT, api(&endpoint, request)).await.unwrap();
    assert!(
        matches!(error, Err(super::ForwardError::HeaderTimeout)),
        "{error:?}"
    );
    serving.await.unwrap();
}

#[tokio::test]
async fn connection_failure_is_not_reported_as_a_header_timeout() {
    let error = api(&Endpoint::Tcp("127.0.0.1:0".into()), get("/"))
        .await
        .unwrap_err();
    assert!(
        matches!(error, super::ForwardError::Unreachable(_)),
        "连接失败仍归 Unreachable，由各调用方按原错误格式处理：{error:?}"
    );
}
