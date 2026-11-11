//! 公网入口和设备入口是两个监听：公开发现不等于公开客户端账号接口。

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::response::IntoResponse;
use hyper::header;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::json;
use tower::ServiceExt;
use wonton_protocol::{WONTON_WELL_KNOWN_PATH, WontonDiscovery};
use wonton_tunnel::relays::{Relay, RelayTable, RelayToken};

use super::fakes::{bind, serve};
use super::tests::{Harness, Reply, TOKEN, owner};
use super::{CORE_COOKIE, Gateway, TunnelPeer, entry_router};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::owner::OwnerStore;
use crate::plugin::languages::Languages;
use crate::tunnel::Tunnel;

// 原样记录上游实际收到的请求，而不是检查路由源码或拼一个假的成功响应。
#[derive(Debug)]
struct Received {
    method: Method,
    uri: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

struct Home {
    app: Router,
    gateway: Arc<Gateway>,
    received: Arc<Mutex<Vec<Received>>>,
    upstream_task: tokio::task::JoinHandle<()>,
    _harness: Harness,
}

impl Home {
    async fn start(matrix_enabled: bool) -> Self {
        let h = Harness::start(None).await;
        let gateway = gateway(&h);
        let received = Arc::new(Mutex::new(Vec::new()));
        let (listener, addr) = bind().await;
        let mut gateway = gateway;
        let matrix = if matrix_enabled {
            let (_fake, mut config) = crate::matrix::fake::start().await;
            let fake_authority = config.url.trim_start_matches("http://").to_owned();
            let http = HttpClient::new();
            let recorded = received.clone();
            let upstream = Router::new().fallback(move |req: Request| {
                let recorded = recorded.clone();
                let http = http.clone();
                let authority = fake_authority.clone();
                async move {
                    let path = req.uri().path();
                    if path.starts_with("/_matrix/federation/") || path.starts_with("/_matrix/key/")
                    {
                        let (parts, body) = req.into_parts();
                        let body = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
                        recorded.lock().unwrap().push(Received {
                            method: parts.method,
                            uri: parts.uri.to_string(),
                            authorization: parts
                                .headers
                                .get(header::AUTHORIZATION)
                                .map(|value| value.to_str().unwrap().to_owned()),
                            body: body.to_vec(),
                        });
                        return (StatusCode::ACCEPTED, axum::Json(json!({"accepted": true})))
                            .into_response();
                    }
                    let (mut parts, body) = req.into_parts();
                    parts.uri = format!("http://{authority}{}", parts.uri).parse().unwrap();
                    let response = http.send(Request::from_parts(parts, body)).await.unwrap();
                    let (parts, body) = response.into_parts();
                    axum::response::Response::from_parts(parts, Body::new(body))
                }
            });
            let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
            config.url = format!("http://{addr}");
            let matrix =
                Matrix::connect(&config, &h.data_dir.join("entry-matrix"), HttpClient::new())
                    .await
                    .unwrap();
            (Some(Arc::new(matrix)), task)
        } else {
            drop(listener);
            (None, tokio::spawn(async {}))
        };
        gateway.matrix = matrix.0;
        let gateway = Arc::new(gateway);
        Self {
            app: entry_router(gateway.clone()),
            gateway,
            received,
            upstream_task: matrix.1,
            _harness: h,
        }
    }

    async fn request(&self, method: Method, host: &str, path: &str) -> Reply {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, host)
            .body(Body::empty())
            .unwrap();
        Reply::of(self.app.clone().oneshot(request).await.unwrap()).await
    }
}

fn gateway(h: &Harness) -> Gateway {
    let profile = OwnerStore::for_test(&h.data_dir.join("entry-owner"), &owner());
    // 不让测试连接任何中继；这里只换发现用的表，复用已有回环 endpoint。
    let relays = RelayTable::from_relays(vec![
        Relay {
            url:
                "https://private:password@relay.homes.test/relay?token=query-secret#fragment-secret"
                    .parse()
                    .unwrap(),
            token: Some(RelayToken::new("relay-admission-secret")),
            tier: 0,
        },
        Relay {
            url: "https://other.homes.test/".parse().unwrap(),
            token: None,
            tier: 1,
        },
    ]);
    let tunnel = Tunnel::new(
        h.tunnel.devices.clone(),
        h.members.clone(),
        Arc::new(|| Languages::from_header(None)),
        h.tunnel.endpoint.clone(),
        0,
        relays,
        tokio::sync::watch::channel(["192.0.2.10".parse().unwrap()].into()).1,
    );
    Gateway::new(
        crate::config::PublicUrls {
            scheme: "http".into(),
            base_domain: "core.homes.test".into(),
            port: 8080,
        },
        profile,
        TOKEN.into(),
        h.members.clone(),
        h.manager.clone(),
        None,
        HttpClient::new(),
        tunnel,
    )
}

#[tokio::test]
async fn discovery_is_local_and_heads_have_identical_metadata_without_a_body() {
    let home = Home::start(true).await;
    let server = home.gateway.matrix.as_ref().unwrap().server_name();
    for (path, expected) in [
        (
            "/.well-known/matrix/server",
            json!({"m.server": format!("{server}:443")}),
        ),
        (
            "/.well-known/matrix/client",
            json!({"m.homeserver": {"base_url": format!("https://{server}")}}),
        ),
        (
            WONTON_WELL_KNOWN_PATH,
            json!({
                "endpoint_id": home.gateway.tunnel.endpoint.id().to_string(),
                "relay_urls": ["https://relay.homes.test/relay", "https://other.homes.test/"]
            }),
        ),
    ] {
        let get = home
            .request(Method::GET, &format!("{}:443", server.to_uppercase()), path)
            .await;
        assert_eq!(get.status, StatusCode::OK);
        assert_eq!(get.json(), expected);
        assert_eq!(get.headers[header::CONTENT_TYPE], "application/json");
        let head = home.request(Method::HEAD, server, path).await;
        assert_eq!(head.status, get.status);
        assert_eq!(
            head.headers[header::CONTENT_TYPE],
            get.headers[header::CONTENT_TYPE]
        );
        assert!(head.text.is_empty());
        assert_eq!(
            head.headers[header::CONTENT_LENGTH].to_str().unwrap(),
            get.text.len().to_string()
        );
        if path == "/.well-known/matrix/client" {
            assert_eq!(get.headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
            assert_eq!(head.headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        }
        let post = home.request(Method::POST, server, path).await;
        assert_matrix_error(&post, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(post.headers[header::ALLOW], "GET, HEAD");
    }
    let discovery: WontonDiscovery = serde_json::from_value(
        home.request(Method::GET, server, WONTON_WELL_KNOWN_PATH)
            .await
            .json(),
    )
    .unwrap();
    assert_eq!(
        discovery.endpoint_id.parse::<iroh::EndpointId>().unwrap(),
        home.gateway.tunnel.endpoint.id()
    );
    assert!(home.received.lock().unwrap().is_empty());
}

fn assert_matrix_error(reply: &Reply, status: StatusCode) {
    assert_eq!(reply.status, status);
    assert_eq!(reply.headers[header::CONTENT_TYPE], "application/json");
    assert!(reply.json()["errcode"].as_str().unwrap().starts_with("M_"));
    assert!(reply.json()["error"].is_string());
}

#[tokio::test]
async fn host_is_exact_and_only_the_default_https_port_is_accepted() {
    let home = Home::start(true).await;
    let server = home.gateway.matrix.as_ref().unwrap().server_name();
    for host in [
        format!("other.{server}"),
        format!("{server}.evil.test"),
        format!("{server}:8448"),
        format!("{server}:0443"),
        format!("{server}."),
        format!("user@{server}:443"),
        String::new(),
    ] {
        assert_matrix_error(
            &home
                .request(Method::GET, &host, "/.well-known/matrix/server")
                .await,
            StatusCode::MISDIRECTED_REQUEST,
        );
    }
    let duplicate = Request::builder()
        .uri("/.well-known/matrix/server")
        .header(header::HOST, server)
        .header(header::HOST, server)
        .body(Body::empty())
        .unwrap();
    assert_matrix_error(
        &Reply::of(home.app.clone().oneshot(duplicate).await.unwrap()).await,
        StatusCode::MISDIRECTED_REQUEST,
    );
    let missing = Request::builder()
        .uri("/.well-known/matrix/server")
        .body(Body::empty())
        .unwrap();
    assert_matrix_error(
        &Reply::of(home.app.clone().oneshot(missing).await.unwrap()).await,
        StatusCode::MISDIRECTED_REQUEST,
    );
    assert!(home.received.lock().unwrap().is_empty());
}

#[tokio::test]
async fn federation_signatures_raw_targets_and_request_bodies_reach_tuwunel_unchanged() {
    let home = Home::start(true).await;
    let server = home.gateway.matrix.as_ref().unwrap().server_name();
    let authorization =
        r#"X-Matrix origin="remote.homes.test",destination="test",key="ed25519:1",sig="a+/b==""#;
    let cases = [
        (
            Method::PUT,
            "/_matrix/federation/v1/send/txn%2FAb?x=%2f%2F&x=2&empty=",
            "{ \"a\":1,\"a\":2 }",
        ),
        (
            Method::POST,
            "/_matrix/key/v2/query?minimum_valid_until_ts=123",
            "{\"server_keys\":{}}",
        ),
        (
            Method::GET,
            "/_matrix/federation/v1/event/$a%2Fb%2Bc?origin=remote.homes.test",
            "",
        ),
    ];
    for (method, path, body) in &cases {
        let req = Request::builder()
            .method(method.clone())
            .uri(*path)
            .header(header::HOST, server)
            .header(header::AUTHORIZATION, authorization)
            .body(Body::from(*body))
            .unwrap();
        let reply = Reply::of(home.app.clone().oneshot(req).await.unwrap()).await;
        assert_eq!(reply.status, StatusCode::ACCEPTED);
    }
    let received = home.received.lock().unwrap();
    assert_eq!(received.len(), cases.len());
    for (received, (method, path, body)) in received.iter().zip(cases) {
        assert_eq!(received.method, method);
        assert_eq!(received.uri, path);
        assert_eq!(received.authorization.as_deref(), Some(authorization));
        assert_eq!(received.body, body.as_bytes());
    }
}

#[tokio::test]
async fn public_entry_never_turns_cookies_owner_tokens_or_tunnel_identity_into_client_access() {
    let home = Home::start(true).await;
    let server = home.gateway.matrix.as_ref().unwrap().server_name();
    let phone = SecretKey::generate().public();
    home.gateway.tunnel.devices.add(phone, None).unwrap();
    // 即使 Matrix client 发现指向服务器名，账号 API 仍只能去已配对设备的私有入口。
    for path in [
        "/_matrix/client/versions",
        "/_matrix/client/v3/login",
        "/_matrix/client/v3/register",
        "/_matrix/client/v3/account/whoami",
        "/_matrix/client/v3/sync",
        "/_matrix/media/v3/upload",
        "/_synapse/admin/v1/register",
        "/_matrix/admin/v1/users",
        "/_wonton/api/status",
        "/_wonton/matrix/login-token",
        "/_matrix/federationevil/v1/version",
        "/_matrix/key",
        "/_matrix/federation",
        "/.well-known/matrix/server/",
    ] {
        let mut req = Request::builder()
            .method(Method::POST)
            .uri(path)
            .header(header::HOST, server)
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(header::COOKIE, format!("{CORE_COOKIE}=not-a-session"))
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(TunnelPeer(phone));
        let reply = Reply::of(home.app.clone().oneshot(req).await.unwrap()).await;
        assert_matrix_error(&reply, StatusCode::NOT_FOUND);
        assert!(!reply.headers.contains_key(header::SET_COOKIE));
    }
    assert!(home.received.lock().unwrap().is_empty());
}

#[tokio::test]
async fn traversal_is_rejected_before_any_upstream_request() {
    let home = Home::start(true).await;
    let server = home.gateway.matrix.as_ref().unwrap().server_name();
    for path in [
        "/_matrix/federation/../client/v3/login",
        "/_matrix/key/./v2/server",
        "/_matrix/federation/%2e%2E/client/v3/login",
        "/_matrix/key/.%2e/admin",
        "/_matrix/key/%2E./admin",
        "/_matrix/key/%2e%2e%2fadmin",
        "/_matrix/key/x%2f..%2fadmin",
        "/_matrix/key/x%5cadmin",
        "/_matrix/federation/\\admin",
        "/_matrix/key/a/..",
    ] {
        assert_matrix_error(
            &home.request(Method::GET, server, path).await,
            StatusCode::BAD_REQUEST,
        );
    }
    assert!(home.received.lock().unwrap().is_empty());
}

#[tokio::test]
async fn missing_and_unreachable_matrix_return_json_not_gateway_html() {
    let missing = Home::start(false).await;
    assert_matrix_error(
        &missing
            .request(Method::GET, "test", "/.well-known/matrix/server")
            .await,
        StatusCode::SERVICE_UNAVAILABLE,
    );
    let mut home = Home::start(true).await;
    home.upstream_task.abort();
    let _ = (&mut home.upstream_task).await;
    let reply = home
        .request(Method::GET, "test", "/_matrix/key/v2/server")
        .await;
    assert_matrix_error(&reply, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), Some("matrix_unavailable"));
}

#[tokio::test]
async fn dedicated_listener_serves_tcp_get_and_head_without_device_identity() {
    let home = Home::start(true).await;
    let (listener, addr) = bind().await;
    serve(listener, home.app.clone()).await;
    for method in [Method::GET, Method::HEAD] {
        let request = Request::builder()
            .method(method.clone())
            .uri(format!("http://{addr}/.well-known/matrix/client"))
            .header(header::HOST, "TEST:443")
            .body(Body::empty())
            .unwrap();
        let response = home.gateway.http.send(request).await.unwrap();
        let reply = Reply::of(response.map(Body::new)).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        if method == Method::HEAD {
            assert!(reply.text.is_empty());
        } else {
            assert_eq!(reply.json()["m.homeserver"]["base_url"], "https://test");
        }
    }
}
