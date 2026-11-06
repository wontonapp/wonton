use std::collections::VecDeque;
use std::sync::Mutex;

use axum::body::Body;
use axum::response::IntoResponse;
use axum::{Json, Router};
use http_body_util::BodyExt;
use hyper::{Request, StatusCode, header};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::*;

fn client(subnets: &[&str], answers: Vec<Vec<SocketAddr>>) -> HttpClient {
    let answers = Arc::new(Mutex::new(VecDeque::from(answers)));
    HttpClient::with_system_connector(SystemConnector {
        subnets: subnets
            .iter()
            .map(|net| Subnet::parse(net).unwrap())
            .collect(),
        resolve: Arc::new(move |_| {
            let answer = answers
                .lock()
                .unwrap()
                .pop_front()
                .expect("每次建连只解析一次");
            Box::pin(async move { Ok(answer) })
        }),
    })
}

fn request(host: &str, port: u16) -> Request<Body> {
    Request::post(format!("http://{host}:{port}/upload"))
        .body(Body::from("private-upload-bytes"))
        .unwrap()
}

#[tokio::test]
async fn outside_and_mixed_answers_send_no_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let outside = listener.local_addr().unwrap();
    let inside = SocketAddr::new("127.0.0.2".parse().unwrap(), outside.port());
    for addresses in [vec![outside], vec![inside, outside]] {
        let http = client(&["127.0.0.2/32"], vec![addresses.clone()]);
        let error = http
            .send(request("tusd.wonton.internal", outside.port()))
            .await
            .unwrap_err();
        match error {
            HttpError::SystemResolution {
                name,
                addresses: found,
            } => {
                assert_eq!(name, "tusd.wonton.internal");
                assert_eq!(found, addresses);
            }
            other => panic!("{other:?}"),
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn empty_or_failed_resolution_cannot_connect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let empty = client(&["127.0.0.0/8"], vec![vec![]]);
    let failed = HttpClient::with_system_connector(SystemConnector {
        subnets: vec![Subnet::parse("127.0.0.0/8").unwrap()].into(),
        resolve: Arc::new(|_| Box::pin(async { Err(io::Error::from(io::ErrorKind::NotFound)) })),
    });
    for http in [empty, failed] {
        assert!(matches!(
            http.send(request("tusd.wonton.internal", port)).await,
            Err(HttpError::Connect(_))
        ));
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
}

async fn serve(listener: TcpListener, marker: &'static str) -> tokio::task::JoinHandle<()> {
    let app = Router::new().fallback(move |req: Request<Body>| async move {
        let host = req.headers()[header::HOST].to_str().unwrap().to_owned();
        let bytes = req.into_body().collect().await.unwrap().to_bytes();
        (
            StatusCode::CREATED,
            [(header::CONNECTION, "close")],
            Json(json!({
                "server": marker, "host": host, "bytes": bytes.as_ref(),
            })),
        )
            .into_response()
    });
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() })
}

#[tokio::test]
async fn each_new_connection_uses_the_new_answer_without_resolving_twice() {
    let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = first.local_addr().unwrap().port();
    let second = TcpListener::bind(("127.0.0.2", port)).await.unwrap();
    let outside = TcpListener::bind(("127.0.0.3", port)).await.unwrap();
    let http = client(
        &["127.0.0.1/32", "127.0.0.2/32"],
        vec![
            vec![first.local_addr().unwrap()],
            vec![second.local_addr().unwrap()],
            vec![outside.local_addr().unwrap()],
        ],
    );
    let tasks = [serve(first, "first").await, serve(second, "rebuilt").await];
    for marker in ["first", "rebuilt"] {
        let response = http
            .send(request("tusd.wonton.internal", port))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            body,
            json!({
                "server": marker,
                "host": format!("tusd.wonton.internal:{port}"),
                "bytes": b"private-upload-bytes".as_slice(),
            })
        );
    }
    assert!(matches!(
        http.send(request("tusd.wonton.internal", port)).await,
        Err(HttpError::SystemResolution { .. })
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), outside.accept())
            .await
            .is_err()
    );
    for task in tasks {
        task.abort();
    }
}

#[tokio::test]
async fn host_development_allows_only_loopback_including_ip_literals() {
    let http = HttpClient::for_system_services(&Engine::unreachable_for_test(), None)
        .await
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = serve(listener, "loopback").await;
    for host in ["localhost", "127.0.0.1", "[::ffff:127.0.0.1]"] {
        let response = http.send(request(host, port)).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        response.into_body().collect().await.unwrap();
    }
    for host in ["192.0.2.10", "[2001:db8::1]", "[::ffff:192.0.2.10]"] {
        assert!(matches!(
            http.send(request(host, port)).await,
            Err(HttpError::SystemResolution { .. })
        ));
    }
    server.abort();
}

#[tokio::test]
async fn ipv6_subnets_are_enforced() {
    let listener = TcpListener::bind("[::1]:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let http = client(&["fd00::/64"], vec![vec![address]]);
    assert!(matches!(
        http.send(request("hydra.wonton.internal", address.port()))
            .await,
        Err(HttpError::SystemResolution { .. })
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
    let server = serve(listener, "ipv6").await;
    let response = client(&["::1/128"], vec![vec![address]])
        .send(request("hydra.wonton.internal", address.port()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    response.into_body().collect().await.unwrap();
    server.abort();
}

async fn docker(
    core_network: Value,
    plugin_network: Value,
) -> (Engine, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let engine = Engine::http_for_test(&format!("http://{}", listener.local_addr().unwrap()));
    let app = Router::new().fallback(move |uri: Uri| {
        let core = core_network.clone();
        let plugin = plugin_network.clone();
        async move {
            let path = uri.path();
            let body = if path.ends_with("/containers/core/json") {
                json!({"Config": {"Labels": {PROJECT: "test-project"}},
                    "NetworkSettings": {"Networks": {"core-net": {}, "plugin-net": {}}}})
            } else if path.ends_with("/networks/core-net") {
                core
            } else if path.ends_with("/networks/plugin-net") {
                plugin
            } else {
                return StatusCode::NOT_FOUND.into_response();
            };
            Json(body).into_response()
        }
    });
    (
        engine,
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }),
    )
}

fn network(project: &str, name: &str, subnets: &[&str]) -> Value {
    json!({"Labels": {PROJECT: project, NETWORK: name},
        "IPAM": {"Config": subnets.iter().map(|s| json!({"Subnet": s})).collect::<Vec<_>>()}})
}

#[tokio::test]
async fn only_the_cores_own_compose_network_is_trusted() {
    let outside = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = outside.local_addr().unwrap().port();
    let inside = TcpListener::bind(("127.0.0.2", port)).await.unwrap();
    let upstream = serve(inside, "core").await;
    for plugin in [
        network("test-project", "plugin", &["127.0.0.1/32"]),
        network("another-project", "core", &["127.0.0.1/32"]),
    ] {
        let (engine, server) = docker(
            network("test-project", "core", &["127.0.0.2/32", "fd00::/64"]),
            plugin,
        )
        .await;
        let http = HttpClient::for_system_services(&engine, Some("core"))
            .await
            .unwrap();
        let response = http.send(request("127.0.0.2", port)).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        response.into_body().collect().await.unwrap();
        assert!(matches!(
            http.send(request("127.0.0.1", port)).await,
            Err(HttpError::SystemResolution { .. })
        ));
        server.abort();
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(50), outside.accept())
            .await
            .is_err()
    );
    upstream.abort();
}

#[tokio::test]
async fn missing_ambiguous_or_unavailable_networks_prevent_startup() {
    for (core, plugin) in [
        (
            network("other", "core", &["127.0.0.2/32"]),
            network("test-project", "plugin", &["127.0.0.1/32"]),
        ),
        (
            network("test-project", "core", &[]),
            network("test-project", "plugin", &["127.0.0.1/32"]),
        ),
        (
            network("test-project", "core", &["127.0.0.2/32"]),
            network("test-project", "core", &["127.0.0.1/32"]),
        ),
    ] {
        let (engine, server) = docker(core, plugin).await;
        assert!(matches!(
            HttpClient::for_system_services(&engine, Some("core")).await,
            Err(EngineError::Rejected { .. })
        ));
        server.abort();
    }
    assert!(matches!(
        HttpClient::for_system_services(&Engine::unreachable_for_test(), Some("core")).await,
        Err(EngineError::Unavailable(_))
    ));
}
