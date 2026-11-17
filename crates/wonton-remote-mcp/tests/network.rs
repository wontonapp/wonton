mod network_support;
#[cfg(target_os = "linux")]
#[path = "network_support/tls_cases.rs"]
mod tls_cases;

use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Redirect;
use axum::routing::get;
use futures_util::stream;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use wonton_remote_mcp::{AddressPolicy, Config, NetworkError};

use network_support::{Answers, Binary, RejectingProxy, client, connector, forward, serve};

const NONPUBLIC: &[&str] = &[
    "0.0.0.0",
    "0.255.255.255",
    "10.0.0.1",
    "100.64.0.0",
    "100.127.255.255",
    "127.0.0.1",
    "169.254.0.1",
    "172.16.0.0",
    "172.31.255.255",
    "192.0.0.1",
    "192.0.2.1",
    "192.88.99.1",
    "192.168.0.1",
    "198.18.0.1",
    "198.19.255.255",
    "198.51.100.1",
    "203.0.113.1",
    "224.0.0.1",
    "239.255.255.255",
    "240.0.0.1",
    "255.255.255.255",
    "::",
    "::1",
    "::2",
    "::127.0.0.1",
    "::ffff:0.0.0.1",
    "::ffff:10.0.0.1",
    "::ffff:100.64.0.1",
    "::ffff:127.0.0.1",
    "::ffff:169.254.0.1",
    "::ffff:172.16.0.1",
    "::ffff:192.168.0.1",
    "::ffff:198.18.0.1",
    "::ffff:224.0.0.1",
    "::ffff:240.0.0.1",
    "64:ff9b::a00:1",
    "64:ff9b:1::1",
    "100::1",
    "2001::1",
    "2001:2::1",
    "2001:10::1",
    "2001:20::1",
    "2001:db8::1",
    "2002:7f00:1::1",
    "3fff::1",
    "fc00::1",
    "fdff::1",
    "fe80::1",
    "febf::1",
    "fec0::1",
    "ff02::1",
    "4000::1",
    "5f00::1",
];

#[tokio::test]
async fn blocks_private_resolutions() {
    let remote = serve(Router::new().route("/mcp", get(|| async { "must not connect" }))).await;
    let client = client();
    for address in NONPUBLIC {
        let forbidden: IpAddr = address.parse().unwrap();
        for addresses in [
            vec!["8.8.8.8".parse().unwrap(), forbidden],
            vec![forbidden, "2606:4700:4700::1111".parse().unwrap()],
        ] {
            let resolver = Answers::new(vec![addresses]);
            let service = connector(Config {
                resolver: resolver.clone(),
                connect_timeout: Duration::from_millis(100),
                ..Config::default()
            })
            .await;
            let target = format!("https://remote.example:{}/mcp", remote.address.port());
            let response = forward(&client, service.address, &target).await;
            let result: Value = response.json().await.unwrap();
            assert_eq!(result["outcome"], "blocked_address", "{address}");
            assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
            assert_eq!(remote.connections.load(Ordering::SeqCst), 0, "{address}");
        }
    }
}

#[tokio::test]
async fn blocks_additional_networks_and_mapped_aliases() {
    let client = client();
    for (address, network) in [
        ("8.8.8.8", "8.8.8.0/24"),
        ("::ffff:8.8.8.8", "8.8.8.0/24"),
        ("::ffff:8.8.8.8", "::ffff:8.8.8.0/120"),
        ("2606:4700:4700::1111", "2606:4700::/32"),
    ] {
        let service = connector(Config {
            resolver: Answers::fixed(address),
            blocked_networks: vec![network.parse().unwrap()],
            ..Config::default()
        })
        .await;
        let response = forward(&client, service.address, "https://remote.example/mcp").await;
        let result: Value = response.json().await.unwrap();
        assert_eq!(result["outcome"], "blocked_address", "{address}");
    }
}

#[tokio::test]
async fn rebinding_rechecked_per_connection() {
    let proxy = RejectingProxy::start().await;
    let resolver = Answers::new(vec![
        vec!["8.8.8.8".parse().unwrap()],
        vec!["127.0.0.1".parse().unwrap()],
    ]);
    let service = connector(Config {
        resolver: resolver.clone(),
        proxy: Some(format!("http://{}", proxy.server.address).parse().unwrap()),
        ..Config::default()
    })
    .await;
    let client = client();
    let first = forward(&client, service.address, "https://remote.example/mcp").await;
    let first: Value = first.json().await.unwrap();
    assert_eq!(first["outcome"], "proxy_unavailable");
    let second = forward(&client, service.address, "https://remote.example/mcp").await;
    let second: Value = second.json().await.unwrap();
    assert_eq!(second["outcome"], "blocked_address");
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    assert_eq!(proxy.server.connections.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn direct_connection_is_not_reused_after_dns_changes() {
    let remote = serve(Router::new().route("/mcp", get(|| async { "first connection" }))).await;
    let resolver = Answers::new(vec![
        vec!["127.0.0.1".parse().unwrap()],
        vec!["127.0.0.2".parse().unwrap()],
    ]);
    let service = connector(Config {
        address_policy: AddressPolicy::AllowPrivate,
        resolver: resolver.clone(),
        blocked_networks: vec!["127.0.0.2/32".parse().unwrap()],
        ..Config::default()
    })
    .await;
    let target = format!("http://remote.example:{}/mcp", remote.address.port());
    let client = client();
    let first = forward(&client, service.address, &target).await;
    assert_eq!(first.text().await.unwrap(), "first connection");
    let second = forward(&client, service.address, &target).await;
    let second: Value = second.json().await.unwrap();
    assert_eq!(second["outcome"], "blocked_address");
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    assert_eq!(remote.connections.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn proxy_prechecks_every_target_answer_before_connecting() {
    let proxy = RejectingProxy::start().await;
    let service = connector(Config {
        resolver: Answers::new(vec![vec![
            "8.8.8.8".parse().unwrap(),
            "127.0.0.1".parse().unwrap(),
        ]]),
        proxy: Some(format!("http://{}", proxy.server.address).parse().unwrap()),
        ..Config::default()
    })
    .await;
    let response = forward(&client(), service.address, "https://remote.example/mcp").await;
    let result: Value = response.json().await.unwrap();
    assert_eq!(result["outcome"], "blocked_address");
    assert_eq!(proxy.server.connections.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn public_ipv4_and_ipv6_proxy_requests_keep_target_hostname() {
    let proxy = RejectingProxy::start().await;
    for address in ["8.8.8.8", "::ffff:8.8.8.8", "2606:4700:4700::1111"] {
        let service = connector(Config {
            resolver: Answers::fixed(address),
            proxy: Some(format!("http://{}", proxy.server.address).parse().unwrap()),
            ..Config::default()
        })
        .await;
        let response = forward(
            &client(),
            service.address,
            "https://remote.example:8443/mcp",
        )
        .await;
        let result: Value = response.json().await.unwrap();
        assert_eq!(result["outcome"], "proxy_unavailable");
    }
    assert_eq!(proxy.server.connections.load(Ordering::SeqCst), 3);
    for request in proxy.requests.lock().unwrap().iter() {
        assert!(request.starts_with("CONNECT remote.example:8443 HTTP/1.1\r\n"));
    }
}

#[tokio::test]
async fn direct_pin_preserves_host_header() {
    let remote = serve(Router::new().route(
        "/mcp",
        get(|headers: HeaderMap| async move { headers["host"].to_str().unwrap().to_owned() }),
    ))
    .await;
    let resolver = Answers::fixed("127.0.0.1");
    let service = connector(Config {
        address_policy: AddressPolicy::AllowPrivate,
        resolver: resolver.clone(),
        ..Config::default()
    })
    .await;
    let target = format!("http://unresolvable.invalid:{}/mcp", remote.address.port());
    let response = forward(&client(), service.address, &target).await;
    assert_eq!(
        response.text().await.unwrap(),
        format!("unresolvable.invalid:{}", remote.address.port())
    );
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(*resolver.hosts.lock().unwrap(), ["unresolvable.invalid"]);
    assert_eq!(remote.connections.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn direct_pin_retains_tls_sni() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (observed, sni) = oneshot::channel();
    let tls = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let hello =
            tokio_rustls::LazyConfigAcceptor::new(rustls::server::Acceptor::default(), socket)
                .await
                .unwrap();
        observed
            .send(hello.client_hello().server_name().unwrap().to_owned())
            .unwrap();
    });
    let service = connector(Config {
        address_policy: AddressPolicy::AllowPrivate,
        resolver: Answers::fixed("127.0.0.1"),
        ..Config::default()
    })
    .await;
    let response = forward(
        &client(),
        service.address,
        &format!("https://tls.example:{port}/mcp"),
    )
    .await;
    let result: Value = response.json().await.unwrap();
    assert_eq!(result["outcome"], "unreachable");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), sni)
            .await
            .unwrap()
            .unwrap(),
        "tls.example"
    );
    tls.await.unwrap();
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let destination = serve(Router::new().route("/mcp", get(|| async { "do not follow" }))).await;
    let location = format!("http://127.0.0.1:{}/mcp", destination.address.port());
    let remote = serve(Router::new().route(
        "/mcp",
        get(move || async move { Redirect::temporary(&location) }),
    ))
    .await;
    let resolver = Answers::fixed("127.0.0.1");
    let service = connector(Config {
        address_policy: AddressPolicy::AllowPrivate,
        resolver: resolver.clone(),
        ..Config::default()
    })
    .await;
    let target = format!("http://remote.example:{}/mcp", remote.address.port());
    let response = forward(&client(), service.address, &target).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(!response.headers().contains_key("location"));
    assert_eq!(
        response.json::<Value>().await.unwrap()["outcome"],
        "remote_error"
    );
    assert_eq!(destination.connections.load(Ordering::SeqCst), 0);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn proxy_down_is_proxy_unavailable() {
    if std::env::var("WONTON_REMOTE_MCP_TEST_ENV").ok().as_deref()
        != Some("proxy_down_is_proxy_unavailable")
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        network_support::isolated_test(
            "proxy_down_is_proxy_unavailable",
            &[("HTTPS_PROXY", &format!("http://{address}"))],
        )
        .await;
        return;
    }
    let mut config = Config::production().unwrap();
    assert!(config.proxy.is_some());
    config.resolver = Answers::fixed("8.8.8.8");
    let service = connector(config).await;
    let response = forward(&client(), service.address, "https://remote.example/mcp").await;
    let result: Value = response.json().await.unwrap();
    assert_eq!(result["outcome"], "proxy_unavailable");
}

#[tokio::test]
async fn main_has_no_relaxed_mode() {
    let remote = serve(Router::new().route("/mcp", get(|| async { "must not connect" }))).await;
    let binary = Binary::start(&[
        ("WONTON_REMOTE_MCP_ALLOW_PRIVATE", "1"),
        ("WONTON_REMOTE_MCP_INSECURE", "1"),
        ("WONTON_REMOTE_MCP_ADDRESS_POLICY", "allow_private"),
        ("WONTON_REMOTE_MCP_RELAXED", "true"),
    ])
    .await;
    let target = format!("https://127.0.0.1:{}/mcp", remote.address.port());
    let response = forward(&client(), binary.address, &target).await;
    let result: Value = response.json().await.unwrap();
    assert_eq!(result["outcome"], "blocked_address");
    assert_eq!(remote.connections.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn production_rejects_invalid_proxy_and_extra_subnet_configuration() {
    for (name, value) in [
        ("HTTPS_PROXY", "not-a-proxy"),
        ("https_proxy", "socks5://127.0.0.1:1080"),
        ("HTTPS_PROXY", "http://127.0.0.1:8080/private?secret=value"),
        (
            "WONTON_REMOTE_MCP_BLOCKED_NETWORKS",
            "10.0.0.0/8,not-a-network",
        ),
        ("WONTON_REMOTE_MCP_BLOCKED_NETWORKS", ""),
    ] {
        let mut child = Binary::command()
            .env("WONTON_REMOTE_MCP_LISTEN", "127.0.0.1:0")
            .env(name, value)
            .spawn()
            .unwrap();
        let exit = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        if exit.is_err() {
            child.kill().unwrap();
            child.wait().unwrap();
        }
        assert!(!exit.unwrap().success(), "{name}");
    }
}

#[tokio::test]
async fn response_headers_deadline_does_not_limit_sse_body() {
    let remote = serve(Router::new().route(
        "/mcp",
        get(|| async {
            let body = Body::from_stream(stream::once(async {
                tokio::time::sleep(Duration::from_millis(250)).await;
                Ok::<_, std::io::Error>("data: eventual event\n\n")
            }));
            ([("content-type", "text/event-stream")], body)
        }),
    ))
    .await;
    let service = connector(Config {
        address_policy: AddressPolicy::AllowPrivate,
        resolver: Answers::fixed("127.0.0.1"),
        connect_timeout: Duration::from_millis(100),
        ..Config::default()
    })
    .await;
    let target = format!("http://remote.example:{}/mcp", remote.address.port());
    let response = forward(&client(), service.address, &target).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "data: eventual event\n\n");
}

#[tokio::test]
async fn response_headers_have_a_deadline() {
    let remote = serve(Router::new().route(
        "/mcp",
        get(|| async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            "too late"
        }),
    ))
    .await;
    let service = connector(Config {
        address_policy: AddressPolicy::AllowPrivate,
        resolver: Answers::fixed("127.0.0.1"),
        connect_timeout: Duration::from_millis(50),
        ..Config::default()
    })
    .await;
    let target = format!("http://remote.example:{}/mcp", remote.address.port());
    let response = tokio::time::timeout(
        Duration::from_secs(1),
        forward(&client(), service.address, &target),
    )
    .await
    .unwrap();
    let result: Value = response.json().await.unwrap();
    assert_eq!(result["outcome"], "unreachable");
}

#[test]
fn classified_errors_never_include_target_or_credentials() {
    for error in [
        NetworkError::BlockedAddress,
        NetworkError::Unreachable,
        NetworkError::ProxyUnavailable,
        NetworkError::InvalidInput,
    ] {
        assert_eq!(error.to_string(), error.detail());
        assert!(!error.detail().contains("https://"));
        assert!(!error.detail().contains("secret"));
    }
    let config = Config::default();
    assert_eq!(config.address_policy, AddressPolicy::PublicOnly);
    assert!(config.proxy.is_none());
    assert_eq!(config.connect_timeout, Duration::from_secs(10));
    assert_eq!(config.probe_timeout, Duration::from_secs(15));
}

#[tokio::test]
async fn default_config_ignores_environment_proxies() {
    if std::env::var("WONTON_REMOTE_MCP_TEST_ENV").ok().as_deref()
        != Some("default_config_ignores_environment_proxies")
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        network_support::isolated_test(
            "default_config_ignores_environment_proxies",
            &[
                ("HTTPS_PROXY", &proxy),
                ("HTTP_PROXY", &proxy),
                ("ALL_PROXY", &proxy),
                ("NO_PROXY", ""),
            ],
        )
        .await;
        return;
    }
    let remote = serve(Router::new().route("/mcp", get(|| async { "direct" }))).await;
    let service = connector(Config {
        address_policy: AddressPolicy::AllowPrivate,
        resolver: Answers::fixed("127.0.0.1"),
        ..Config::default()
    })
    .await;
    let target = format!("http://unresolvable.invalid:{}/mcp", remote.address.port());
    let response = forward(&client(), service.address, &target).await;
    assert_eq!(response.text().await.unwrap(), "direct");
}

#[tokio::test]
async fn production_reads_lowercase_proxy_and_extra_networks() {
    if std::env::var("WONTON_REMOTE_MCP_TEST_ENV").ok().as_deref()
        != Some("production_reads_lowercase_proxy_and_extra_networks")
    {
        network_support::isolated_test(
            "production_reads_lowercase_proxy_and_extra_networks",
            &[
                ("https_proxy", "http://127.0.0.1:12345"),
                (
                    "WONTON_REMOTE_MCP_BLOCKED_NETWORKS",
                    "8.8.8.0/24, 2606:4700::/32",
                ),
            ],
        )
        .await;
        return;
    }
    let mut config = Config::production().unwrap();
    assert_eq!(config.address_policy, AddressPolicy::PublicOnly);
    assert_eq!(
        config.proxy.as_ref().unwrap().as_str(),
        "http://127.0.0.1:12345/"
    );
    assert_eq!(config.blocked_networks.len(), 2);
    config.resolver = Answers::fixed("8.8.8.8");
    let service = connector(config).await;
    let response = forward(&client(), service.address, "https://remote.example/mcp").await;
    let result: Value = response.json().await.unwrap();
    assert_eq!(result["outcome"], "blocked_address");
}

#[tokio::test]
async fn compressed_response_bytes_are_not_modified() {
    const GZIP: &[u8] = &[
        31, 139, 8, 0, 0, 0, 0, 0, 0, 3, 203, 72, 205, 201, 201, 7, 0, 134, 166, 16, 54, 5, 0, 0, 0,
    ];
    let remote = serve(Router::new().route(
        "/mcp",
        get(|| async { ([("content-encoding", "gzip")], GZIP) }),
    ))
    .await;
    let service = connector(Config {
        address_policy: AddressPolicy::AllowPrivate,
        resolver: Answers::fixed("127.0.0.1"),
        ..Config::default()
    })
    .await;
    let target = format!("http://remote.example:{}/mcp", remote.address.port());
    let response = forward(&client(), service.address, &target).await;
    assert_eq!(response.headers()["content-encoding"], "gzip");
    assert_eq!(response.bytes().await.unwrap().as_ref(), GZIP);
}

#[tokio::test]
async fn dns_has_a_deadline_and_errors_are_redacted() {
    struct PendingResolver;
    impl wonton_remote_mcp::Resolver for PendingResolver {
        fn resolve(&self, _host: &str) -> wonton_remote_mcp::ResolveFuture {
            Box::pin(std::future::pending())
        }
    }
    let service = connector(Config {
        resolver: std::sync::Arc::new(PendingResolver),
        connect_timeout: Duration::from_millis(50),
        ..Config::default()
    })
    .await;
    let response = tokio::time::timeout(
        Duration::from_secs(1),
        forward(
            &client(),
            service.address,
            "https://sensitive.example/url-secret?token=query-secret",
        ),
    )
    .await
    .unwrap();
    let text = response.text().await.unwrap();
    let result: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(result["outcome"], "unreachable");
    for secret in ["sensitive.example", "url-secret", "query-secret"] {
        assert!(!text.contains(secret));
    }
}

#[tokio::test]
async fn strict_urls_require_https_and_domain_without_credentials() {
    let resolver = Answers::fixed("127.0.0.1");
    let service = connector(Config {
        resolver: resolver.clone(),
        ..Config::default()
    })
    .await;
    for target in [
        "http://remote.example/mcp",
        "https://127.0.0.1/mcp",
        "https://[::1]/mcp",
        "https://[::ffff:127.0.0.1]/mcp",
        "https://localhost/mcp",
        "https://remote.localhost/mcp",
        "https://remote.localhost./mcp",
        "https://single-label/mcp",
        "https://username:password@remote.example/mcp",
        "https://remote.example/mcp#secret-fragment",
    ] {
        let response = forward(&client(), service.address, target).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{target}");
    }
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
}
