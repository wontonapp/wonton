use std::sync::atomic::Ordering;
use std::time::Duration;

use base64::Engine;
use serde_json::Value;
use wonton_remote_mcp::{AddressPolicy, Config};

use crate::network_support::tls::{ProxyMode, TargetMode, TlsFixture, Trace};
use crate::network_support::{Answers, client, connector, isolated_test};

#[tokio::test]
async fn https_proxy_rejects_certificate_before_connect() {
    let Some(trace) = run_case(
        "tls_cases::https_proxy_rejects_certificate_before_connect",
        ProxyMode::WrongCertificate,
        TargetMode::Eof,
        Some("proxy_unavailable"),
    )
    .await
    else {
        return;
    };
    assert!(trace.proxy_tls_rejected);
    assert!(!trace.proxy_tls_accepted);
    assert!(trace.connect_request.is_none());
    assert!(trace.target_sni.is_none());
}

#[tokio::test]
async fn http_proxy_target_tls_eof_is_unreachable() {
    let Some(trace) = run_case(
        "tls_cases::http_proxy_target_tls_eof_is_unreachable",
        ProxyMode::Http,
        TargetMode::Eof,
        Some("unreachable"),
    )
    .await
    else {
        return;
    };
    assert_tunnel(&trace, false);
    assert!(!trace.target_tls_accepted);
    assert!(!trace.target_tls_rejected);
}

#[tokio::test]
async fn https_proxy_target_tls_eof_is_unreachable() {
    let Some(trace) = run_case(
        "tls_cases::https_proxy_target_tls_eof_is_unreachable",
        ProxyMode::Https,
        TargetMode::Eof,
        Some("unreachable"),
    )
    .await
    else {
        return;
    };
    assert_tunnel(&trace, true);
    assert!(!trace.target_tls_accepted);
    assert!(!trace.target_tls_rejected);
}

#[tokio::test]
async fn http_proxy_target_certificate_is_unreachable() {
    let Some(trace) = run_case(
        "tls_cases::http_proxy_target_certificate_is_unreachable",
        ProxyMode::Http,
        TargetMode::WrongCertificate,
        Some("unreachable"),
    )
    .await
    else {
        return;
    };
    assert_tunnel(&trace, false);
    assert!(trace.target_tls_rejected);
    assert!(!trace.target_tls_accepted);
}

#[tokio::test]
async fn https_proxy_target_certificate_is_unreachable() {
    let Some(trace) = run_case(
        "tls_cases::https_proxy_target_certificate_is_unreachable",
        ProxyMode::Https,
        TargetMode::WrongCertificate,
        Some("unreachable"),
    )
    .await
    else {
        return;
    };
    assert_tunnel(&trace, true);
    assert!(trace.target_tls_rejected);
    assert!(!trace.target_tls_accepted);
}

#[tokio::test]
async fn healthy_https_proxy_and_target_verify_certificates() {
    let Some(trace) = run_case(
        "tls_cases::healthy_https_proxy_and_target_verify_certificates",
        ProxyMode::Https,
        TargetMode::Healthy,
        None,
    )
    .await
    else {
        return;
    };
    assert_tunnel(&trace, true);
    assert!(trace.target_tls_accepted);
    assert!(!trace.target_tls_rejected);
    assert_target_request(trace.target_request.as_deref().unwrap());
}

#[tokio::test]
async fn direct_target_certificate_is_unreachable() {
    let Some(trace) = run_case(
        "tls_cases::direct_target_certificate_is_unreachable",
        ProxyMode::Direct,
        TargetMode::WrongCertificate,
        Some("unreachable"),
    )
    .await
    else {
        return;
    };
    assert!(trace.connect_request.is_none());
    assert_eq!(trace.target_sni.as_deref(), Some("tls-target.example"));
    assert!(trace.target_tls_rejected);
    assert!(!trace.target_tls_accepted);
}

#[tokio::test]
async fn direct_target_verifies_certificate_and_retains_sni() {
    let Some(trace) = run_case(
        "tls_cases::direct_target_verifies_certificate_and_retains_sni",
        ProxyMode::Direct,
        TargetMode::Healthy,
        None,
    )
    .await
    else {
        return;
    };
    assert!(trace.connect_request.is_none());
    assert_eq!(trace.target_sni.as_deref(), Some("tls-target.example"));
    assert!(trace.target_tls_accepted);
    assert!(!trace.target_tls_rejected);
    assert_target_request(trace.target_request.as_deref().unwrap());
}

async fn run_case(
    name: &str,
    proxy_mode: ProxyMode,
    target_mode: TargetMode,
    expected: Option<&str>,
) -> Option<Trace> {
    if std::env::var("WONTON_REMOTE_MCP_TEST_ENV").ok().as_deref() == Some(name) {
        request_case(proxy_mode, expected).await;
        return None;
    }
    let mut fixture = TlsFixture::start(proxy_mode, target_mode).await;
    let proxy = match proxy_mode {
        ProxyMode::Direct => String::new(),
        ProxyMode::Http => format!(
            "http://proxy-user:proxy-credential@{}",
            fixture.server.address
        ),
        ProxyMode::Https | ProxyMode::WrongCertificate => format!(
            "https://proxy-user:proxy-credential@{}",
            fixture.server.address
        ),
    };
    let port = if matches!(proxy_mode, ProxyMode::Direct) {
        fixture.server.address.port()
    } else {
        8443
    };
    let target = format!("https://tls-target.example:{port}/sensitive-path?token=sensitive-query");
    // The locked Linux platform verifier reads SSL_CERT_FILE. Only this child
    // process trusts the fixture CA; certificate and hostname checks stay on.
    isolated_test(
        name,
        &[
            ("SSL_CERT_FILE", fixture.ca_file.to_str().unwrap()),
            (
                "SSL_CERT_DIR",
                fixture.ca_file.parent().unwrap().to_str().unwrap(),
            ),
            ("WONTON_REMOTE_MCP_TLS_PROXY", &proxy),
            ("WONTON_REMOTE_MCP_TLS_TARGET", &target),
        ],
    )
    .await;
    let trace = fixture.trace().await;
    assert_eq!(fixture.server.connections.load(Ordering::SeqCst), 1);
    Some(trace)
}

async fn request_case(proxy_mode: ProxyMode, expected: Option<&str>) {
    let direct = matches!(proxy_mode, ProxyMode::Direct);
    let resolver = Answers::fixed(if direct { "127.0.0.1" } else { "8.8.8.8" });
    let proxy = std::env::var("WONTON_REMOTE_MCP_TLS_PROXY").unwrap();
    let target = std::env::var("WONTON_REMOTE_MCP_TLS_TARGET").unwrap();
    let service = connector(Config {
        address_policy: if direct {
            AddressPolicy::AllowPrivate
        } else {
            AddressPolicy::PublicOnly
        },
        resolver: resolver.clone(),
        proxy: (!proxy.is_empty()).then(|| proxy.parse().unwrap()),
        connect_timeout: Duration::from_secs(2),
        ..Config::default()
    })
    .await;
    let encoded = base64::engine::general_purpose::STANDARD
        .encode(r#"{"authorization":"Bearer forward-credential"}"#);
    let response = client()
        .get(format!("http://{}/mcp", service.address))
        .header("x-wonton-plugin", "network-tls-contract")
        .header("x-wonton-remote-url", &target)
        .header("x-wonton-remote-auth", "header")
        .header("x-wonton-remote-forward", &encoded)
        .send()
        .await
        .unwrap();
    let body = response.text().await.unwrap();
    if let Some(outcome) = expected {
        let result: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(result["outcome"], outcome);
        for sensitive in [
            "https://",
            "http://",
            "tls-target.example",
            "sensitive-path",
            "sensitive-query",
            "proxy-user",
            "proxy-credential",
            "forward-credential",
            "127.0.0.1",
            &encoded,
        ] {
            assert!(!body.contains(sensitive));
        }
    } else {
        assert_eq!(body, "tls-ok");
    }
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
}

fn assert_tunnel(trace: &Trace, tls: bool) {
    assert_eq!(trace.proxy_tls_accepted, tls);
    assert!(!trace.proxy_tls_rejected);
    assert_eq!(trace.target_sni.as_deref(), Some("tls-target.example"));
    let request = trace.connect_request.as_deref().unwrap();
    assert!(request.starts_with("CONNECT tls-target.example:8443 HTTP/1.1\r\n"));
    assert!(!request.contains("sensitive-path"));
    assert!(!request.contains("sensitive-query"));
    let credential =
        base64::engine::general_purpose::STANDARD.encode("proxy-user:proxy-credential");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("proxy-authorization: basic ")
    );
    assert!(request.contains(&credential));
}

fn assert_target_request(request: &str) {
    assert!(request.starts_with("GET /sensitive-path?token=sensitive-query HTTP/1.1\r\n"));
    assert!(request.contains("Bearer forward-credential"));
    assert!(!request.to_ascii_lowercase().contains("proxy-authorization"));
    assert!(!request.contains("proxy-credential"));
}
