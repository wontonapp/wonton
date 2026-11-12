use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::Query;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tokio::task::JoinHandle;

use super::*;

const CONTENT: &[u8] = b"{\"schemaVersion\":2,\"manifests\":[]}";
const WAIT: Duration = Duration::from_secs(2);

struct Server {
    url: String,
    task: JoinHandle<()>,
}

impl Server {
    async fn start(router: Router) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self { url, task }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn digest() -> String {
    format!("sha256:{:x}", Sha256::digest(CONTENT))
}

fn challenge(realm: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [("www-authenticate", format!(
            "Bearer realm=\"{realm}\",service=\"registry.test\",scope=\"repository:gogost/gost:pull\""
        ))],
    )
        .into_response()
}

fn assert_failure(result: RegistryProbe, reason: RegistryFailure) -> RegistryProblem {
    assert!(result.probed_at_ms > 0);
    assert!(result.latency_ms.is_none());
    let problem = result.problem.unwrap();
    assert_eq!(problem.reason, reason);
    problem
}

async fn bearer_flow(token_field: &'static str) {
    let tokens = Arc::new(AtomicUsize::new(0));
    let manifests = Arc::new(AtomicUsize::new(0));
    let token_calls = tokens.clone();
    let auth_server = Server::start(Router::new().route(
        "/token",
        get(
            move |headers: HeaderMap, Query(query): Query<HashMap<String, String>>| {
                let calls = token_calls.clone();
                async move {
                    assert!(!headers.contains_key("authorization"));
                    assert_eq!(query.get("service").unwrap(), "registry.test");
                    assert_eq!(query.get("scope").unwrap(), "repository:gogost/gost:pull");
                    calls.fetch_add(1, Ordering::SeqCst);
                    axum::Json(serde_json::json!({token_field: "anonymous-token"}))
                }
            },
        ),
    ))
    .await;
    let realm = format!("{}/token", auth_server.url);
    let manifest_calls = manifests.clone();
    let server = Server::start(Router::new().route(
        "/manifest",
        get(move |headers: HeaderMap| {
            let realm = realm.clone();
            let calls = manifest_calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                assert!(
                    headers["user-agent"]
                        .to_str()
                        .unwrap()
                        .starts_with("docker/")
                );
                assert!(
                    headers["accept"]
                        .to_str()
                        .unwrap()
                        .contains("application/vnd.oci.image.index.v1+json")
                );
                assert_eq!(headers["accept-encoding"], "identity");
                match headers.get("authorization") {
                    None => challenge(&realm),
                    Some(token) if token == "Bearer anonymous-token" => CONTENT.into_response(),
                    _ => StatusCode::FORBIDDEN.into_response(),
                }
            }
        }),
    ))
    .await;
    let result = probe_url(
        &format!("{}/manifest", server.url),
        &digest(),
        WAIT,
        false,
        &Default::default(),
    )
    .await;
    assert!(result.problem.is_none(), "{result:?}");
    assert!(result.latency_ms.is_some());
    assert!(result.probed_at_ms > 0);
    assert_eq!(tokens.load(Ordering::SeqCst), 1);
    assert_eq!(manifests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn registry_probe_fetches_anonymous_token() {
    bearer_flow("token").await;
    bearer_flow("access_token").await;
}

#[tokio::test]
async fn registry_probe_preserves_rejected_http_statuses() {
    for status in [
        StatusCode::FORBIDDEN,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::BAD_GATEWAY,
    ] {
        let server =
            Server::start(Router::new().route("/manifest", get(move || async move { status })))
                .await;
        let result = probe_url(
            &format!("{}/manifest", server.url),
            &digest(),
            WAIT,
            false,
            &Default::default(),
        )
        .await;
        let problem = assert_failure(result, RegistryFailure::Rejected);
        assert!(problem.message.contains(&status.as_u16().to_string()));
    }
}

#[tokio::test]
async fn registry_probe_retries_authentication_only_once() {
    let requests = Arc::new(AtomicUsize::new(0));
    let tokens = Server::start(Router::new().route(
        "/token",
        get(|| async { axum::Json(serde_json::json!({"token": "not-accepted"})) }),
    ))
    .await;
    let realm = format!("{}/token", tokens.url);
    let calls = requests.clone();
    let server = Server::start(Router::new().route(
        "/manifest",
        get(move || {
            let realm = realm.clone();
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                challenge(&realm)
            }
        }),
    ))
    .await;
    let result = probe_url(
        &format!("{}/manifest", server.url),
        &digest(),
        WAIT,
        false,
        &Default::default(),
    )
    .await;
    let problem = assert_failure(result, RegistryFailure::Rejected);
    assert!(problem.message.contains("401"));
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn registry_probe_reports_token_endpoint_rejection() {
    let tokens =
        Server::start(Router::new().route("/token", get(|| async { StatusCode::FORBIDDEN }))).await;
    let realm = format!("{}/token", tokens.url);
    let server = Server::start(Router::new().route(
        "/manifest",
        get(move || {
            let realm = realm.clone();
            async move { challenge(&realm) }
        }),
    ))
    .await;
    let result = probe_url(
        &format!("{}/manifest", server.url),
        &digest(),
        WAIT,
        false,
        &Default::default(),
    )
    .await;
    let problem = assert_failure(result, RegistryFailure::Rejected);
    assert!(problem.message.contains("403"));
}

#[tokio::test]
async fn registry_probe_checks_raw_response_digest() {
    let server =
        Server::start(Router::new().route("/manifest", get(|| async { "wrong content" }))).await;
    let result = probe_url(
        &format!("{}/manifest", server.url),
        &digest(),
        WAIT,
        false,
        &Default::default(),
    )
    .await;
    assert_failure(result, RegistryFailure::DigestMismatch);
}

#[tokio::test]
async fn registry_probe_does_not_decompress_manifest() {
    use std::io::Write;

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(CONTENT).unwrap();
    let encoded = encoder.finish().unwrap();
    let expected = format!("sha256:{:x}", Sha256::digest(&encoded));
    let server = Server::start(Router::new().route(
        "/manifest",
        get(move || {
            let encoded = encoded.clone();
            async move { ([("content-encoding", "gzip")], encoded) }
        }),
    ))
    .await;
    let result = probe_url(
        &format!("{}/manifest", server.url),
        &expected,
        WAIT,
        false,
        &Default::default(),
    )
    .await;
    assert!(result.problem.is_none(), "{result:?}");
}

#[tokio::test]
async fn registry_probe_total_timeout_includes_token_body() {
    let tokens = Server::start(Router::new().route(
        "/token",
        get(|| async {
            let body = Body::from_stream(futures_util::stream::once(async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok::<_, std::io::Error>(Bytes::from_static(b"{\"token\":\"late\"}"))
            }));
            Response::new(body)
        }),
    ))
    .await;
    let realm = format!("{}/token", tokens.url);
    let server = Server::start(Router::new().route(
        "/manifest",
        get(move || {
            let realm = realm.clone();
            async move { challenge(&realm) }
        }),
    ))
    .await;
    let result = probe_url(
        &format!("{}/manifest", server.url),
        &digest(),
        Duration::from_millis(100),
        false,
        &Default::default(),
    )
    .await;
    assert_failure(result, RegistryFailure::Unreachable);
}

#[tokio::test]
async fn registry_probe_total_timeout_includes_manifest_body() {
    let server = Server::start(Router::new().route(
        "/manifest",
        get(|| async {
            Response::new(Body::from_stream(futures_util::stream::once(async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok::<_, std::io::Error>(Bytes::from_static(CONTENT))
            })))
        }),
    ))
    .await;
    let result = probe_url(
        &format!("{}/manifest", server.url),
        &digest(),
        Duration::from_millis(100),
        false,
        &Default::default(),
    )
    .await;
    assert_failure(result, RegistryFailure::Unreachable);
}

#[tokio::test]
async fn registry_probe_redirect_strips_authorization() {
    let received = Arc::new(AtomicUsize::new(0));
    let calls = received.clone();
    let destination = Server::start(Router::new().route(
        "/manifest",
        get(move |headers: HeaderMap| {
            let calls = calls.clone();
            async move {
                assert!(!headers.contains_key("authorization"));
                calls.fetch_add(1, Ordering::SeqCst);
                CONTENT
            }
        }),
    ))
    .await;
    let token_server = Server::start(Router::new().route(
        "/token",
        get(|headers: HeaderMap| async move {
            assert!(!headers.contains_key("authorization"));
            axum::Json(serde_json::json!({"token": "private-token"}))
        }),
    ))
    .await;
    let realm = format!("{}/token", token_server.url);
    let destination_url = format!(
        "{}/manifest",
        destination.url.replace("127.0.0.1", "localhost")
    );
    let server = Server::start(Router::new().route(
        "/manifest",
        get(move |headers: HeaderMap| {
            let realm = realm.clone();
            let destination = destination_url.clone();
            async move {
                if headers.contains_key("authorization") {
                    (StatusCode::TEMPORARY_REDIRECT, [("location", destination)]).into_response()
                } else {
                    challenge(&realm)
                }
            }
        }),
    ))
    .await;
    let result = probe_url(
        &format!("{}/manifest", server.url),
        &digest(),
        WAIT,
        false,
        &Default::default(),
    )
    .await;
    assert!(result.problem.is_none(), "{result:?}");
    assert_eq!(received.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn registry_probe_authenticates_only_the_challenging_redirect_target() {
    let token_server = Server::start(Router::new().route(
        "/token",
        get(|headers: HeaderMap| async move {
            assert!(!headers.contains_key("authorization"));
            axum::Json(serde_json::json!({"token": "target-token"}))
        }),
    ))
    .await;
    let realm = format!("{}/token", token_server.url);
    let destination = Server::start(Router::new().route(
        "/manifest",
        get(move |headers: HeaderMap| {
            let realm = realm.clone();
            async move {
                if let Some(token) = headers.get("authorization") {
                    assert_eq!(token, "Bearer target-token");
                    CONTENT.into_response()
                } else {
                    challenge(&realm)
                }
            }
        }),
    ))
    .await;
    let target = format!("{}/manifest", destination.url);
    let requests = Arc::new(AtomicUsize::new(0));
    let calls = requests.clone();
    let origin = Server::start(Router::new().route(
        "/manifest",
        get(move |headers: HeaderMap| {
            let target = target.clone();
            let calls = calls.clone();
            async move {
                assert!(!headers.contains_key("authorization"));
                calls.fetch_add(1, Ordering::SeqCst);
                (StatusCode::TEMPORARY_REDIRECT, [("location", target)])
            }
        }),
    ))
    .await;
    let result = probe_url(
        &format!("{}/manifest", origin.url),
        &digest(),
        WAIT,
        false,
        &Default::default(),
    )
    .await;
    assert!(result.problem.is_none(), "{result:?}");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}
