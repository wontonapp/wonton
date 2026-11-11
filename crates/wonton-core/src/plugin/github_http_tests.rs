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
use crate::plugin::languages::Languages;

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
        let data = github
            .file(&repo(), COMMIT, "icon.png", 100, &Languages::default())
            .await
            .unwrap();
        assert_eq!(data, b"complete");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn a_source_that_stays_slow_stops_after_two_attempts_with_a_typed_timeout() {
    for answer in [Answer::Headers, Answer::Body] {
        let (github, calls) = source(answer, false).await;
        let error = github
            .file(&repo(), COMMIT, "icon.png", 100, &Languages::default())
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
            github
                .file(&repo(), reference, "icon.png", 100, &Languages::default())
                .await,
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
            .file(&repo(), COMMIT, "icon.png", max, &Languages::default())
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
        .file(&repo(), COMMIT, "icon.png", 100, &Languages::default())
        .await
        .unwrap_err();
    assert!(matches!(&error, GitHubError::Unavailable(_)), "{error}");
    assert_eq!(github_error(error).kind(), "source_unavailable");
}

/// 127.0.0.2 上 backlog 为 0 的监听器（后续 SYN 停住）和 127.0.0.1 上同端口的正常监听器。
///
/// reqwest 的 resolve_to_addrs 不看地址里的端口，只用 URL 里的，两个地址必须共用一个端口；
/// 而临时端口只保证在绑定它的地址上空闲。所以先在 127.0.0.1:0 挑出端口，再试 127.0.0.2 上的
/// 同一端口，撞了就把两个都关掉重挑，最多 20 次。两个监听器一起活着交回：先关再重绑的话，
/// 空档里端口可能又被并行的测试或别的进程占走。
#[cfg(target_os = "linux")]
async fn stalled_and_healthy_listeners() -> (TcpListener, TcpListener) {
    use std::io::ErrorKind;
    use std::net::{Ipv4Addr, SocketAddr};
    use tokio::net::TcpSocket;

    for _ in 0..20 {
        let healthy = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = healthy.local_addr().unwrap().port();
        let stalled_address = SocketAddr::from(([127, 0, 0, 2], port));
        let socket = TcpSocket::new_v4().unwrap();
        match socket.bind(stalled_address) {
            Ok(()) => return (socket.listen(0).unwrap(), healthy),
            Err(error) if error.kind() == ErrorKind::AddrInUse => continue,
            Err(error) => panic!("绑定 {stalled_address} 失败: {error}"),
        }
    }
    panic!("20 次都没挑到 127.0.0.1 和 127.0.0.2 上同时空闲的端口");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_port_taken_on_localhost_is_not_picked() {
    use std::net::Ipv4Addr;
    use tokio::net::TcpStream;

    let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let (stalled, healthy) = stalled_and_healthy_listeners().await;
    assert_ne!(
        healthy.local_addr().unwrap().port(),
        taken.local_addr().unwrap().port()
    );
    // 两个地址落在同一个端口，且监听器活着、能连上。
    assert_eq!(
        stalled.local_addr().unwrap().port(),
        healthy.local_addr().unwrap().port()
    );
    TcpStream::connect(healthy.local_addr().unwrap())
        .await
        .unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_stalled_first_ipv4_does_not_hide_the_next_address() {
    use tokio::net::TcpStream;

    let (stalled, healthy) = stalled_and_healthy_listeners().await;
    let address = stalled.local_addr().unwrap();
    // Linux backlog 为零仍容纳一个已连接请求；不 accept，让后续 SYN 等待而不是立即被拒。
    let _queued = TcpStream::connect(address).await.unwrap();
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
        github.file(&repo(), COMMIT, "icon.png", 100, &Languages::default()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(data, b"complete");
}
