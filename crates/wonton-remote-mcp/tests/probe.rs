mod probe_support;

use std::{
    io,
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::http::{Method, StatusCode};
use probe_support::{Options, Server, config, initialize, probe, simple};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use wonton_remote_mcp::{Config, ResolveFuture, Resolver};

async fn successful_handshake(sse: bool, delete_status: StatusCode) {
    let remote = Server::remote(Options {
        sse,
        delete_status,
        ..Options::default()
    })
    .await;
    let result = simple(&remote).await;
    assert_eq!(result["outcome"], "ok", "{result}");
    assert_eq!(result["http_status"], 200);
    assert_eq!(result["tools"], json!(["search"]));
    let requests = remote.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].body["method"], "initialize");
    assert_eq!(requests[0].body["params"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        requests[0].headers["accept"],
        "application/json, text/event-stream"
    );
    assert!(!requests[0].headers.contains_key("mcp-session-id"));
    assert_eq!(requests[1].body["method"], "notifications/initialized");
    assert!(requests[1].body.get("id").is_none());
    assert_eq!(requests[2].body["method"], "tools/list");
    assert_eq!(requests[3].method, Method::DELETE);
    for request in &requests[1..] {
        assert_eq!(request.headers["mcp-session-id"], "probe-session");
        assert_eq!(request.headers["mcp-protocol-version"], "2025-06-18");
        assert!(
            !request
                .headers
                .keys()
                .any(|name| name.as_str().starts_with("x-wonton-"))
        );
    }
}

#[tokio::test]
async fn probe_ok_json() {
    successful_handshake(false, StatusCode::NO_CONTENT).await;
}

#[tokio::test]
async fn probe_ok_sse() {
    successful_handshake(true, StatusCode::NO_CONTENT).await;
}

#[tokio::test]
async fn probe_delete_405_is_accepted() {
    successful_handshake(false, StatusCode::METHOD_NOT_ALLOWED).await;
}

#[tokio::test]
async fn probe_without_tools_capability_skips_tools_list() {
    let mut value = initialize();
    value["result"]["capabilities"] = json!({});
    let remote = Server::remote(Options {
        initialize: value,
        ..Options::default()
    })
    .await;
    let result = simple(&remote).await;
    assert_eq!(result["outcome"], "ok");
    assert!(result.get("tools").is_none());
    let requests = remote.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1].body["method"], "notifications/initialized");
    assert_eq!(requests[2].method, Method::DELETE);
}

#[tokio::test]
async fn probe_stateless_remote_does_not_delete_unnegotiated_session() {
    let remote = Server::remote(Options {
        session: false,
        ..Options::default()
    })
    .await;
    assert_eq!(simple(&remote).await["outcome"], "ok");
    let requests = remote.requests();
    assert_eq!(requests.len(), 3);
    assert!(
        !requests
            .iter()
            .any(|request| request.method == Method::DELETE)
    );
    assert!(
        requests
            .iter()
            .all(|request| !request.headers.contains_key("mcp-session-id"))
    );
}

#[tokio::test]
async fn probe_401_resource_metadata_needs_oauth() {
    let remote = Server::remote(Options {
        status: StatusCode::UNAUTHORIZED,
        challenge: Some("Basic realm=\"other\", Bearer realm=\"mcp,remote\", resource_metadata=\"https://auth.example.com/metadata\"".to_owned()),
        ..Options::default()
    }).await;
    let result = simple(&remote).await;
    assert_eq!(result["outcome"], "needs_oauth");
    assert_eq!(result["http_status"], 401);
    assert_eq!(remote.requests().len(), 1);
}

#[tokio::test]
async fn probe_401_wellknown_only_needs_oauth() {
    let remote = Server::remote(Options {
        status: StatusCode::UNAUTHORIZED,
        well_known: Some("/.well-known/oauth-protected-resource".to_owned()),
        ..Options::default()
    })
    .await;
    assert_eq!(simple(&remote).await["outcome"], "needs_oauth");
    let paths: Vec<_> = remote
        .requests()
        .iter()
        .map(|request| request.uri.path().to_owned())
        .collect();
    assert_eq!(
        paths,
        [
            "/mcp",
            "/.well-known/oauth-protected-resource/mcp",
            "/.well-known/oauth-protected-resource"
        ]
    );
}

#[tokio::test]
async fn probe_401_path_wellknown_needs_oauth() {
    let remote = Server::remote(Options {
        status: StatusCode::UNAUTHORIZED,
        well_known: Some("/.well-known/oauth-protected-resource/mcp".to_owned()),
        ..Options::default()
    })
    .await;
    assert_eq!(simple(&remote).await["outcome"], "needs_oauth");
    assert_eq!(remote.requests().len(), 2);
}

#[tokio::test]
async fn probe_401_bare_needs_header_secret() {
    let remote = Server::remote(Options {
        status: StatusCode::UNAUTHORIZED,
        ..Options::default()
    })
    .await;
    let result = simple(&remote).await;
    assert_eq!(result["outcome"], "needs_header_secret");
    assert_eq!(result["http_status"], 401);
    assert_eq!(remote.requests().len(), 3);
}

#[tokio::test]
async fn probe_401_invalid_metadata_does_not_claim_oauth() {
    for metadata in [
        json!({}),
        json!({"resource":"https://other.example.com/mcp"}),
        json!({"resource":42}),
    ] {
        let remote = Server::remote(Options {
            status: StatusCode::UNAUTHORIZED,
            challenge: Some(
                "Basic resource_metadata=\"https://auth.example.com/metadata\"".to_owned(),
            ),
            well_known: Some("/.well-known/oauth-protected-resource/mcp".to_owned()),
            metadata: Some(metadata),
            ..Options::default()
        })
        .await;
        assert_eq!(simple(&remote).await["outcome"], "needs_header_secret");
    }
}

#[tokio::test]
async fn probe_401_with_forwarded_header_is_secret_rejected() {
    let remote = Server::remote(Options {
        status: StatusCode::UNAUTHORIZED,
        challenge: Some(
            "Bearer resource_metadata=\"https://auth.example.com/metadata\"".to_owned(),
        ),
        ..Options::default()
    })
    .await;
    let result = probe(
        &remote.url,
        config(),
        "header",
        json!({"Authorization":"Bearer secret-test-value"}),
    )
    .await;
    assert_eq!(result["outcome"], "secret_rejected");
    assert_eq!(remote.requests().len(), 1);
    assert_eq!(
        remote.requests()[0].headers["authorization"],
        "Bearer secret-test-value"
    );
    assert!(!result.to_string().contains("secret-test-value"));
}

#[tokio::test]
async fn probe_403_with_forwarded_header_is_secret_rejected() {
    let remote = Server::remote(Options {
        status: StatusCode::FORBIDDEN,
        ..Options::default()
    })
    .await;
    let result = probe(
        &remote.url,
        config(),
        "header",
        json!({"X-Api-Key":"secret-test-value"}),
    )
    .await;
    assert_eq!(result["outcome"], "secret_rejected");
    assert_eq!(result["http_status"], 403);
    assert_eq!(remote.requests().len(), 1);
}

#[tokio::test]
async fn probe_html_not_mcp() {
    let remote = Server::remote(Options {
        html: true,
        ..Options::default()
    })
    .await;
    let result = simple(&remote).await;
    assert_eq!(result["outcome"], "not_mcp");
    assert!(!result.to_string().contains("private-body-secret"));
    assert_eq!(remote.requests().len(), 1);
}

#[tokio::test]
async fn probe_405_then_endpoint_event_is_legacy_sse() {
    let remote = Server::remote(Options {
        status: StatusCode::METHOD_NOT_ALLOWED,
        get_body: ": heartbeat\r\n\r\nevent: endpoint\r\ndata: /messages?sessionId=secret-test-value\r\n\r\n".to_owned(),
        ..Options::default()
    }).await;
    let result = simple(&remote).await;
    assert_eq!(result["outcome"], "legacy_sse");
    assert!(!result.to_string().contains("secret-test-value"));
    let requests = remote.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].method, Method::GET);
    assert_eq!(requests[1].headers["accept"], "text/event-stream");
}

#[tokio::test]
async fn probe_404_then_no_endpoint_not_mcp() {
    let remote = Server::remote(Options {
        status: StatusCode::NOT_FOUND,
        ..Options::default()
    })
    .await;
    assert_eq!(simple(&remote).await["outcome"], "not_mcp");
    assert_eq!(remote.requests().len(), 2);
}

#[tokio::test]
async fn probe_legacy_requires_first_noncomment_event() {
    let remote = Server::remote(Options {
        status: StatusCode::BAD_REQUEST,
        get_body: "data: {}\n\nevent: endpoint\ndata: /messages\n\n".to_owned(),
        ..Options::default()
    })
    .await;
    assert_eq!(simple(&remote).await["outcome"], "not_mcp");
}

#[tokio::test]
async fn probe_503_remote_error() {
    let remote = Server::remote(Options {
        status: StatusCode::SERVICE_UNAVAILABLE,
        ..Options::default()
    })
    .await;
    let result = simple(&remote).await;
    assert_eq!(result["outcome"], "remote_error");
    assert_eq!(result["http_status"], 503);
    assert_eq!(remote.requests().len(), 1);
}

#[tokio::test]
async fn probe_malformed_initialize_is_never_ok() {
    let valid = initialize();
    let mut wrong_id = valid.clone();
    wrong_id["id"] = json!(77);
    let mut wrong_jsonrpc = valid.clone();
    wrong_jsonrpc["jsonrpc"] = json!("1.0");
    let mut unsupported = valid.clone();
    unsupported["result"]["protocolVersion"] = json!("2099-01-01");
    let malformed = [
        json!({}),
        json!({"jsonrpc":"2.0","id":1,"result":{}}),
        json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{}}}),
        json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","serverInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"private-body-secret"}}),
        wrong_id,
        wrong_jsonrpc,
        unsupported,
    ];
    for sse in [false, true] {
        for initialize in malformed.clone() {
            let remote = Server::remote(Options {
                initialize,
                sse,
                ..Options::default()
            })
            .await;
            let result = simple(&remote).await;
            assert_eq!(result["outcome"], "remote_error", "{result}");
            assert!(!result.to_string().contains("private-body-secret"));
            let requests = remote.requests();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[1].method, Method::DELETE);
            assert_eq!(requests[1].headers["mcp-session-id"], "probe-session");
        }
    }
}

#[tokio::test]
async fn probe_later_handshake_failures_are_reported_and_cleaned_up() {
    let cases = [
        Options {
            notification_status: StatusCode::BAD_REQUEST,
            ..Options::default()
        },
        Options {
            tools_status: StatusCode::SERVICE_UNAVAILABLE,
            ..Options::default()
        },
        Options {
            tools: json!({"jsonrpc":"2.0","id":2,"result":{}}),
            ..Options::default()
        },
        Options {
            delete_status: StatusCode::SERVICE_UNAVAILABLE,
            ..Options::default()
        },
    ];
    for options in cases {
        let remote = Server::remote(options).await;
        let result = simple(&remote).await;
        assert_eq!(result["outcome"], "remote_error", "{result}");
        assert!(result.get("tools").is_none());
        let requests = remote.requests();
        let cleanup = requests.last().unwrap();
        assert_eq!(cleanup.method, Method::DELETE);
        assert_eq!(cleanup.headers["mcp-session-id"], "probe-session");
        assert_eq!(cleanup.headers["mcp-protocol-version"], "2025-06-18");
    }
}

#[tokio::test]
async fn probe_rejects_reserved_headers_before_contacting_remote() {
    let remote = Server::remote(Options::default()).await;
    for header in [
        "Host",
        "X-Wonton-Plugin",
        "Connection",
        "Content-Length",
        "Mcp-Session-Id",
        "MCP-Protocol-Version",
    ] {
        let mut headers = serde_json::Map::new();
        headers.insert(header.to_owned(), json!("private-header-value"));
        let result = probe(&remote.url, config(), "header", Value::Object(headers)).await;
        assert_eq!(result["outcome"], "invalid", "{header}: {result}");
        assert!(!result.to_string().contains("private-header-value"));
    }
    assert!(remote.requests().is_empty());
}

struct FailingResolver;
impl Resolver for FailingResolver {
    fn resolve(&self, _: &str) -> ResolveFuture {
        Box::pin(async {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "private-dns-message",
            ))
        })
    }
}

#[tokio::test]
async fn probe_dns_failure_unreachable() {
    let config = Config {
        resolver: Arc::new(FailingResolver),
        ..Config::default()
    };
    let result = probe(
        "https://remote.example.com/private-url-secret",
        config,
        "none",
        json!({}),
    )
    .await;
    assert_eq!(result["outcome"], "unreachable");
    assert!(!result.to_string().contains("private-dns-message"));
    assert!(!result.to_string().contains("private-url-secret"));
    assert!(!result.to_string().contains("remote.example.com"));
}

#[tokio::test]
async fn probe_tls_failure_unreachable() {
    // A real TCP listener serving plaintext HTTP cannot complete a TLS handshake.
    let remote = Server::remote(Options::default()).await;
    let url = remote.url.replacen("http://", "https://", 1);
    let result = probe(&url, config(), "none", json!({})).await;
    assert_eq!(result["outcome"], "unreachable");
}

#[tokio::test]
async fn probe_connection_refused_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/mcp?token=private-url-secret",
        listener.local_addr().unwrap()
    );
    drop(listener);
    let result = probe(&url, config(), "none", json!({})).await;
    assert_eq!(result["outcome"], "unreachable");
    assert!(!result.to_string().contains("private-url-secret"));
}

#[tokio::test]
async fn probe_timeout_unreachable() {
    let remote = Server::remote(Options {
        delay_method: Some("initialize"),
        ..Options::default()
    })
    .await;
    let mut config = config();
    config.probe_timeout = Duration::from_millis(150);
    let started = Instant::now();
    let result = probe(&remote.url, config, "none", json!({})).await;
    assert_eq!(result["outcome"], "unreachable");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn probe_deadline_covers_response_body() {
    let remote = Server::remote(Options {
        hanging_body: true,
        ..Options::default()
    })
    .await;
    let mut config = config();
    config.probe_timeout = Duration::from_millis(150);
    let started = Instant::now();
    let result = probe(&remote.url, config, "none", json!({})).await;
    assert_eq!(result["outcome"], "unreachable");
    assert!(started.elapsed() < Duration::from_secs(2));
    let cleanup = remote.requests().pop().unwrap();
    assert_eq!(cleanup.method, Method::DELETE);
    assert_eq!(cleanup.headers["mcp-session-id"], "probe-session");
}

#[tokio::test]
async fn probe_deadline_is_shared_by_all_handshake_steps() {
    let remote = Server::remote(Options {
        delay_all: Duration::from_millis(60),
        ..Options::default()
    })
    .await;
    let mut config = config();
    config.probe_timeout = Duration::from_millis(150);
    let result = probe(&remote.url, config, "none", json!({})).await;
    assert_eq!(result["outcome"], "unreachable");
    assert!(remote.requests().len() >= 2);
}

#[tokio::test]
async fn probe_proxy_down_is_proxy_unavailable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    drop(listener);
    let config = Config {
        proxy: Some(proxy),
        resolver: Arc::new(PublicResolver),
        ..Config::default()
    };
    let result = probe(
        "https://remote.example.com/private-url-secret",
        config,
        "none",
        json!({}),
    )
    .await;
    assert_eq!(result["outcome"], "proxy_unavailable");
    assert!(!result.to_string().contains("private-url-secret"));
}

struct PublicResolver;
impl Resolver for PublicResolver {
    fn resolve(&self, _: &str) -> ResolveFuture {
        Box::pin(async { Ok(vec![IpAddr::from([93, 184, 216, 34])]) })
    }
}

#[tokio::test]
async fn probe_deadline_covers_sse_body() {
    let remote = Server::remote(Options {
        sse: true,
        hanging_body: true,
        ..Options::default()
    })
    .await;
    let mut config = config();
    config.probe_timeout = Duration::from_millis(150);
    let started = Instant::now();
    let result = probe(&remote.url, config, "none", json!({})).await;
    assert_eq!(result["outcome"], "unreachable");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(remote.requests().last().unwrap().method, Method::DELETE);
}

#[tokio::test]
async fn probe_deadline_covers_session_delete() {
    let remote = Server::remote(Options {
        delay_method: Some("DELETE"),
        ..Options::default()
    })
    .await;
    let mut config = config();
    config.probe_timeout = Duration::from_millis(150);
    let started = Instant::now();
    let result = probe(&remote.url, config, "none", json!({})).await;
    assert_eq!(result["outcome"], "unreachable");
    assert_eq!(remote.requests().last().unwrap().method, Method::DELETE);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn probe_wellknown_drops_endpoint_query() {
    let remote = Server::remote(Options {
        status: StatusCode::UNAUTHORIZED,
        ..Options::default()
    })
    .await;
    let url = format!("{}?token=private-url-secret", remote.url);
    let result = probe(&url, config(), "none", json!({})).await;
    assert_eq!(result["outcome"], "needs_header_secret");
    assert!(!result.to_string().contains("private-url-secret"));
    let requests = remote.requests();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[1..]
            .iter()
            .all(|request| request.uri.query().is_none())
    );
}

#[tokio::test]
async fn probe_rejects_unexpanded_url_secret() {
    let remote = Server::remote(Options::default()).await;
    let result = probe(
        &format!("{}?key={{secret:api_key}}", remote.url),
        config(),
        "none",
        json!({}),
    )
    .await;
    assert_eq!(result["outcome"], "invalid");
    assert!(remote.requests().is_empty());
}

#[tokio::test]
async fn probe_invalid_json_or_sse_initialize_is_remote_error() {
    for (sse, raw) in [
        (false, "{invalid-json"),
        (true, "event: message\ndata: {invalid-json\n\n"),
    ] {
        let remote = Server::remote(Options {
            sse,
            raw_initialize: Some(raw),
            ..Options::default()
        })
        .await;
        let result = simple(&remote).await;
        assert_eq!(result["outcome"], "remote_error");
        assert!(result.get("tools").is_none());
        assert_eq!(remote.requests().last().unwrap().method, Method::DELETE);
    }
}

#[tokio::test]
async fn probe_deadline_still_deletes_negotiated_session() {
    let cases = [
        Options {
            delay_method: Some("notifications/initialized"),
            ..Options::default()
        },
        Options {
            delay_method: Some("tools/list"),
            ..Options::default()
        },
        Options {
            hanging_tools_body: true,
            ..Options::default()
        },
        Options {
            hanging_tools_body: true,
            sse: true,
            ..Options::default()
        },
    ];
    for options in cases {
        let remote = Server::remote(options).await;
        let mut config = config();
        config.probe_timeout = Duration::from_millis(150);
        let result = probe(&remote.url, config, "none", json!({})).await;
        assert_eq!(result["outcome"], "unreachable");
        assert!(result.get("tools").is_none());
        let requests = remote.requests();
        let cleanup = requests.last().unwrap();
        assert_eq!(cleanup.method, Method::DELETE);
        assert_eq!(cleanup.headers["mcp-session-id"], "probe-session");
        assert_eq!(cleanup.headers["mcp-protocol-version"], "2025-06-18");
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.method == Method::DELETE)
                .count(),
            1
        );
    }
}
