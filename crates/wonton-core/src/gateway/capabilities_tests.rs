//! 走真实 TCP 入口，能力授权仍由 PluginManager 的实际声明、连接和服务状态检查决定。

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::Request;
use http_body_util::BodyExt;
use hyper::{Method, StatusCode, header};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, oneshot};

use super::tests::Harness;
use crate::plugin::languages::Languages;
use crate::plugin::upstreams::Endpoint;
use crate::plugin::{PluginId, PluginPackage, PluginStatus};

const WAIT: Duration = Duration::from_secs(5);

fn manifest(id: &str, declarations: &str) -> crate::plugin::manifest::Manifest {
    PluginPackage::parse(
        &format!(
            "services:\n  api: {{image: example/api}}\nx-wonton:\n  spec: '0.11'\n  id: {id}\n  \
             version: 1.0.0\n  kind: app\n  title: {{en: Test}}\n  description: {{en: Test}}\n  \
             permissions: []\n{declarations}\n"
        ),
        &Languages::from_header(None),
    )
    .unwrap()
    .manifest
}

fn register(h: &Harness, capability: &str, declarations: &str) {
    h.manager.register_for_test(
        manifest("app.consumer", declarations),
        PluginStatus::Running,
        None,
    );
    h.manager.register_for_test(
        manifest(
            "app.provider",
            &format!("  provides:\n    {capability}: {{service: api, port: 8080}}"),
        ),
        PluginStatus::Running,
        None,
    );
    h.manager.serving_for_test(&provider(), &["api"]);
}

fn provider() -> PluginId {
    PluginId::parse("app.provider").unwrap()
}

fn connect(h: &Harness, capability: &str, endpoint: Endpoint) {
    h.manager.capability_route_for_test(
        &PluginId::parse("app.consumer").unwrap(),
        capability,
        &provider(),
        endpoint,
    );
}

fn source(h: &Harness) {
    h.manager
        .register_source_for_test("app.consumer", "127.0.0.0/8", Vec::new());
}

fn request(h: &Harness, capability: &str, path: &str, body: Body) -> Request {
    Request::builder()
        .method(Method::POST)
        .uri(format!("http://{}{path}", h.addr))
        .header(header::HOST, format!("{capability}.wonton.internal:8080"))
        .header("x-wonton-plugin", "app.spoofed")
        .header(header::AUTHORIZATION, "Bearer consumer-key")
        .body(body)
        .unwrap()
}

async fn rejects(h: &Harness, capability: &str, status: StatusCode, kind: &str) {
    let response = h
        .http
        .send(request(h, capability, "/api?key=value", Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()[super::ERROR_HEADER], kind);
}

#[tokio::test]
async fn requests_require_an_authenticated_declared_and_approved_connection() {
    let h = Harness::start(None).await;
    register(&h, "torrent_client", "  requires: [torrent_client]");
    rejects(
        &h,
        "torrent_client",
        StatusCode::FORBIDDEN,
        "unauthenticated",
    )
    .await;
    source(&h);
    rejects(
        &h,
        "torrent_client",
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;

    connect(&h, "torrent_client", Endpoint::Tcp(h.upstream.to_string()));
    h.manager
        .register_for_test(manifest("app.consumer", ""), PluginStatus::Running, None);
    rejects(
        &h,
        "torrent_client",
        StatusCode::FORBIDDEN,
        "permission_denied",
    )
    .await;
    assert_eq!(h.hits.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn approved_requests_report_a_stopped_or_unhealthy_provider() {
    let h = Harness::start(None).await;
    register(&h, "torrent_client", "  wants: [torrent_client]");
    source(&h);
    connect(&h, "torrent_client", Endpoint::Tcp(h.upstream.to_string()));
    h.manager.serving_for_test(&provider(), &[]);
    rejects(
        &h,
        "torrent_client",
        StatusCode::SERVICE_UNAVAILABLE,
        "dependency_stopped",
    )
    .await;
    h.manager.register_for_test(
        manifest(
            "app.provider",
            "  provides:\n    torrent_client: {service: api, port: 8080}",
        ),
        PluginStatus::Stopped,
        None,
    );
    h.manager.serving_for_test(&provider(), &["api"]);
    rejects(
        &h,
        "torrent_client",
        StatusCode::SERVICE_UNAVAILABLE,
        "dependency_stopped",
    )
    .await;
    assert_eq!(h.hits.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn arbitrary_internal_hosts_never_become_capability_routes() {
    let h = Harness::start(None).await;
    source(&h);
    for capability in ["unknown", "x-acme.invalid_name", "egress_proxy"] {
        rejects(&h, capability, StatusCode::NOT_FOUND, "unknown_host").await;
    }
}

#[derive(Debug)]
struct Received {
    method: Method,
    path: String,
    caller: Vec<String>,
    credential: String,
    first: Bytes,
}

fn caller_bot(h: &Harness) -> wonton_protocol::BotRef {
    let id = PluginId::parse("app.consumer").unwrap();
    h.manager.bot_for_test(&id, "@consumer:test");
    wonton_protocol::BotRef {
        plugin_id: id.to_string(),
        bot: "default".into(),
    }
}

#[tokio::test]
async fn bot_headers_are_validated_before_routing_and_forwarded_once() {
    let app = Router::new().fallback(|req: Request| async move {
        assert_eq!(req.headers()["x-wonton-plugin"], "app.consumer");
        let bots: Vec<_> = req.headers().get_all("x-wonton-bot").iter().collect();
        assert!(bots.len() <= 1);
        bots.first()
            .map_or("none".to_owned(), |bot| bot.to_str().unwrap().to_owned())
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::Tcp(listener.local_addr().unwrap().to_string());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let h = Harness::start(None).await;
    register(&h, "torrent_client", "  wants: [torrent_client]");
    source(&h);
    connect(&h, "torrent_client", endpoint.clone());
    caller_bot(&h);
    for name in [
        None,
        Some("unknown"),
        Some("deleted"),
        Some("default, unknown"),
    ] {
        let mut req = request(&h, "torrent_client", "/", Body::empty());
        if let Some(name) = name {
            req.headers_mut()
                .insert("x-wonton-bot", name.parse().unwrap());
        }
        let response = h.http.send(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            "none"
        );
    }
    let mut req = request(&h, "torrent_client", "/", Body::empty());
    req.headers_mut()
        .insert("x-wonton-bot", "default".parse().unwrap());
    let response = h.http.send(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "default"
    );
    let mut req = request(&h, "torrent_client", "/", Body::empty());
    req.headers_mut()
        .insert("x-wonton-bot", " default ".parse().unwrap());
    req.headers_mut().insert(
        header::CONNECTION,
        "X-Wonton-Bot, X-Wonton-Plugin".parse().unwrap(),
    );
    let response = h.http.send(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "default"
    );
    for second in ["default", "unknown"] {
        let mut req = request(&h, "torrent_client", "/", Body::empty());
        req.headers_mut()
            .append("x-wonton-bot", "default".parse().unwrap());
        req.headers_mut()
            .append("x-wonton-bot", second.parse().unwrap());
        assert_eq!(
            h.http.send(req).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    h.manager
        .bot_records_for_test(&PluginId::parse("app.consumer").unwrap(), &[]);
    let mut req = request(&h, "torrent_client", "/", Body::empty());
    req.headers_mut()
        .insert("x-wonton-bot", "default".parse().unwrap());
    let response = h.http.send(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "none"
    );
}

async fn streaming_provider(
    observed: oneshot::Sender<Received>,
    release: oneshot::Receiver<()>,
) -> String {
    let state = Arc::new(Mutex::new(Some((observed, release))));
    let app = Router::new().fallback(move |req: Request| {
        let state = state.clone();
        async move {
            let (observed, release) = state.lock().await.take().unwrap();
            let (parts, mut body) = req.into_parts();
            let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
            observed
                .send(Received {
                    method: parts.method,
                    path: parts.uri.path_and_query().unwrap().as_str().to_owned(),
                    caller: parts
                        .headers
                        .get_all("x-wonton-plugin")
                        .iter()
                        .map(|value| value.to_str().unwrap().to_owned())
                        .collect(),
                    credential: parts.headers[header::AUTHORIZATION]
                        .to_str()
                        .unwrap()
                        .to_owned(),
                    first,
                })
                .unwrap();
            assert_eq!(body.collect().await.unwrap().to_bytes(), "second");
            let first = futures_util::stream::once(async {
                Ok::<_, std::convert::Infallible>(Bytes::from("data: first\n\n"))
            });
            let last = futures_util::stream::once(async move {
                release.await.unwrap();
                Ok(Bytes::from("data: last\n\n"))
            });
            Body::from_stream(futures_util::StreamExt::chain(first, last))
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr.to_string()
}

#[tokio::test]
async fn third_party_requests_preserve_path_credentials_identity_and_bidirectional_streaming() {
    let (observed, received) = oneshot::channel();
    let (release_response, released_response) = oneshot::channel();
    let endpoint = streaming_provider(observed, released_response).await;
    let h = Harness::start(None).await;
    register(&h, "x-acme.ledger", "  wants: [x-acme.ledger]");
    source(&h);
    connect(&h, "x-acme.ledger", Endpoint::Tcp(endpoint));
    let (release_upload, released_upload) = oneshot::channel();
    let first = futures_util::stream::once(async {
        Ok::<_, std::convert::Infallible>(Bytes::from("first"))
    });
    let last = futures_util::stream::once(async move {
        released_upload.await.unwrap();
        Ok(Bytes::from("second"))
    });
    let body = Body::from_stream(futures_util::StreamExt::chain(first, last));
    let mut req = request(&h, "x-acme.ledger", "/api/%2Fentry?x=a%2Fb&x=two", body);
    req.headers_mut()
        .append("x-wonton-plugin", "app.second-spoof".parse().unwrap());
    req.headers_mut().insert(
        header::CONNECTION,
        "X-Wonton-Plugin, keep-alive".parse().unwrap(),
    );
    let http = h.http.clone();
    let pending = tokio::spawn(async move { http.send(req).await.unwrap() });
    let received = tokio::time::timeout(WAIT, received).await.unwrap().unwrap();
    assert_eq!(received.method, Method::POST);
    assert_eq!(received.path, "/api/%2Fentry?x=a%2Fb&x=two");
    assert_eq!(received.caller, ["app.consumer"]);
    assert_eq!(received.credential, "Bearer consumer-key");
    assert_eq!(received.first, "first");
    release_upload.send(()).unwrap();
    let response = tokio::time::timeout(WAIT, pending).await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let first = tokio::time::timeout(WAIT, body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first.into_data().unwrap(), "data: first\n\n");
    release_response.send(()).unwrap();
    assert_eq!(body.collect().await.unwrap().to_bytes(), "data: last\n\n");
}

async fn websocket_provider() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let app = Router::new().fallback(|mut req: Request| async move {
        assert_eq!(
            req.uri().path_and_query().unwrap().as_str(),
            "/socket?channel=a%2Fb"
        );
        assert_eq!(req.headers()["x-wonton-plugin"], "app.consumer");
        assert_eq!(req.headers()[header::AUTHORIZATION], "Bearer consumer-key");
        assert_eq!(req.headers()[header::UPGRADE], "websocket");
        let upgrade = hyper::upgrade::on(&mut req);
        assert_eq!(req.headers()["x-wonton-bot"], "default");
        assert_eq!(req.headers().get_all("x-wonton-bot").iter().count(), 1);
        tokio::spawn(async move {
            let mut socket = hyper_util::rt::TokioIo::new(upgrade.await.unwrap());
            // 客户端发带掩码的文本帧 hi；提供方返回不同的确认帧，不做回显。
            let mut frame = [0; 8];
            socket.read_exact(&mut frame).await.unwrap();
            assert_eq!(frame, [0x81, 0x82, 1, 2, 3, 4, b'i', b'k']);
            socket.write_all(b"\x81\x08accepted").await.unwrap();
        });
        axum::response::Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header(header::CONNECTION, "upgrade")
            .header(header::UPGRADE, "websocket")
            .header("sec-websocket-accept", "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=")
            .body(Body::empty())
            .unwrap()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr.to_string()
}

#[tokio::test]
async fn capability_websocket_upgrades_keep_authenticated_identity_and_exchange_frames() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let endpoint = websocket_provider().await;
    let h = Harness::start(None).await;
    register(&h, "torrent_client", "  requires: [torrent_client]");
    source(&h);
    let bot = caller_bot(&h);
    h.manager.capability_bot_route_for_test(
        &bot,
        "torrent_client",
        &provider(),
        None,
        Endpoint::Tcp(endpoint),
    );
    tokio::time::timeout(WAIT, async {
        let mut socket = tokio::net::TcpStream::connect(h.addr).await.unwrap();
        socket
            .write_all(
                b"GET /socket?channel=a%2Fb HTTP/1.1\r\nHost: torrent_client.wonton.internal\r\n\
                  Connection: Upgrade, X-Wonton-Plugin, X-Wonton-Bot\r\nUpgrade: websocket\r\n\
                  Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                  X-Wonton-Bot: default\r\nX-Wonton-Plugin: app.spoofed\r\nAuthorization: Bearer consumer-key\r\n\r\n",
            )
            .await
            .unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(socket.read_u8().await.unwrap());
        }
        assert!(
            head.starts_with(b"HTTP/1.1 101"),
            "{}",
            String::from_utf8_lossy(&head)
        );
        socket
            .write_all(&[0x81, 0x82, 1, 2, 3, 4, b'i', b'k'])
            .await
            .unwrap();
        let mut frame = [0; 10];
        socket.read_exact(&mut frame).await.unwrap();
        assert_eq!(&frame, b"\x81\x08accepted");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn corrupt_bot_storage_never_falls_back_to_the_plugin_provider() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let app = Router::new().fallback(move || {
        let seen = seen.clone();
        async move {
            seen.fetch_add(1, Ordering::SeqCst);
            "provider"
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::Tcp(listener.local_addr().unwrap().to_string());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let h = Harness::start(None).await;
    register(&h, "torrent_client", "  wants: [torrent_client]");
    source(&h);
    let other = PluginId::parse("app.other").unwrap();
    h.manager.register_for_test(
        manifest(
            other.as_str(),
            "  provides:\n    torrent_client: {service: api, port: 8080}",
        ),
        PluginStatus::Running,
        None,
    );
    h.manager.serving_for_test(&other, &["api"]);
    let consumer = PluginId::parse("app.consumer").unwrap();
    h.manager
        .capability_route_for_test(&consumer, "torrent_client", &other, endpoint.clone());
    let bot = caller_bot(&h);
    h.manager
        .capability_bot_route_for_test(&bot, "torrent_client", &provider(), None, endpoint);
    assert_eq!(
        h.manager
            .capability_route(
                &consumer,
                "torrent_client",
                &Languages::from_header(None),
                Some(&bot)
            )
            .await
            .unwrap()
            .provider,
        provider()
    );
    let path = h.data_dir.join("plugins/bots/app.consumer.json");
    let saved = std::fs::read(&path).unwrap();
    std::fs::write(&path, b"{").unwrap();
    let mut req = request(&h, "torrent_client", "/", Body::empty());
    req.headers_mut()
        .insert("x-wonton-bot", "default".parse().unwrap());
    let response = h.http.send(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.headers()[super::ERROR_HEADER],
        "dependency_stopped"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        h.manager
            .capability_route(
                &consumer,
                "torrent_client",
                &Languages::from_header(None),
                Some(&bot)
            )
            .await,
        Err(crate::plugin::dependencies::CapabilityAccessError::Unavailable(_))
    ));
    std::fs::write(&path, saved).unwrap();
    let mut req = request(&h, "torrent_client", "/", Body::empty());
    req.headers_mut()
        .insert("x-wonton-bot", "default".parse().unwrap());
    assert_eq!(h.http.send(req).await.unwrap().status(), StatusCode::OK);
    assert_eq!(
        h.manager
            .capability_route(
                &consumer,
                "torrent_client",
                &Languages::from_header(None),
                Some(&bot)
            )
            .await
            .unwrap()
            .provider,
        provider()
    );
}
