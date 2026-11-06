use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tokio::net::TcpListener;

use super::{CoreProxy, GitHub, GitHubError, Repo, client};
use crate::plugin::github_source::github_error;

const LIMIT: Duration = Duration::from_millis(300);
const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[derive(Clone, Copy)]
enum Answer {
    Headers,
    Body,
    Status(StatusCode),
    Oversized,
}

async fn source(answer: Answer, recover: bool) -> (GitHub, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let app = Router::new().fallback(move || {
        let number = seen.fetch_add(1, Ordering::SeqCst);
        async move {
            if recover && number > 0 {
                return "complete".into_response();
            }
            match answer {
                Answer::Headers => std::future::pending::<Response>().await,
                Answer::Body => {
                    let chunks = futures_util::stream::once(async {
                        Ok::<_, Infallible>(Bytes::from_static(b"partial"))
                    });
                    let body =
                        futures_util::StreamExt::chain(chunks, futures_util::stream::pending());
                    Body::from_stream(body).into_response()
                }
                Answer::Status(status) => status.into_response(),
                Answer::Oversized => "too large".into_response(),
            }
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut github = GitHub::for_test(&base);
    github.client = client(&CoreProxy::default())
        .timeout(LIMIT)
        .build()
        .unwrap();
    (github, calls)
}

fn repo() -> Repo {
    Repo {
        owner: "acme".into(),
        name: "plugin".into(),
    }
}

#[tokio::test]
async fn a_pinned_file_recovers_from_a_stalled_header_or_partial_body() {
    for answer in [Answer::Headers, Answer::Body] {
        let (github, calls) = source(answer, true).await;
        let data = github.file(&repo(), COMMIT, "icon.png", 100).await.unwrap();
        assert_eq!(data, b"complete");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn a_source_that_stays_slow_stops_after_two_attempts_with_a_typed_timeout() {
    for answer in [Answer::Headers, Answer::Body] {
        let (github, calls) = source(answer, false).await;
        let error = github
            .file(&repo(), COMMIT, "icon.png", 100)
            .await
            .unwrap_err();
        assert!(matches!(&error, GitHubError::Timeout(_)), "{error}");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let error = github_error(error);
        assert_eq!(error.kind(), "source_unavailable");
    }
}

#[tokio::test]
async fn a_mutable_reference_is_not_replayed_after_a_timeout() {
    for reference in ["HEAD", "main", "v1.0.0"] {
        let (github, calls) = source(Answer::Headers, true).await;
        assert!(matches!(
            github.file(&repo(), reference, "icon.png", 100).await,
            Err(GitHubError::Timeout(_))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn rejections_and_invalid_files_are_not_replayed() {
    for (answer, max) in [
        (Answer::Status(StatusCode::NOT_FOUND), 100),
        (Answer::Status(StatusCode::SERVICE_UNAVAILABLE), 100),
        (Answer::Oversized, 2),
    ] {
        let (github, calls) = source(answer, true).await;
        let error = github
            .file(&repo(), COMMIT, "icon.png", max)
            .await
            .unwrap_err();
        assert!(!matches!(error, GitHubError::Timeout(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn a_refused_connection_is_not_a_response_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let github = GitHub::for_test(&base);
    let error = github
        .file(&repo(), COMMIT, "icon.png", 100)
        .await
        .unwrap_err();
    assert!(matches!(&error, GitHubError::Unavailable(_)), "{error}");
    assert_eq!(github_error(error).kind(), "source_unavailable");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_stalled_first_ipv4_does_not_hide_the_next_address() {
    use tokio::net::{TcpSocket, TcpStream};

    let socket = TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.2:0".parse().unwrap()).unwrap();
    let stalled = socket.listen(0).unwrap();
    let address = stalled.local_addr().unwrap();
    // Linux backlog 为零仍容纳一个已连接请求；不 accept，让后续 SYN 等待而不是立即被拒。
    let _queued = TcpStream::connect(address).await.unwrap();
    let healthy = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, address.port()))
        .await
        .unwrap();
    tokio::spawn(async move {
        axum::serve(healthy, Router::new().fallback(|| async { "complete" }))
            .await
            .unwrap();
    });
    let base = format!("http://source.test:{}", address.port());
    let mut github = GitHub::for_test(&base);
    github.client = client(&CoreProxy::default())
        .resolve_to_addrs(
            "source.test",
            &[address, ([127, 0, 0, 1], address.port()).into()],
        )
        .build()
        .unwrap();
    let data = tokio::time::timeout(
        Duration::from_secs(10),
        github.file(&repo(), COMMIT, "icon.png", 100),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(data, b"complete");
}
