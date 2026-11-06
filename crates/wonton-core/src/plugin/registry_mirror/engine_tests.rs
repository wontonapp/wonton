use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::json;

use super::reference::mirrored;
use crate::plugin::compose::PullPolicy;
use crate::plugin::docker::{Engine, EngineError};

const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[derive(Clone, Default)]
struct RegistryDocker {
    pulls: Arc<Mutex<Vec<String>>>,
    reject: bool,
    digest: Option<String>,
}

async fn serve(State(state): State<RegistryDocker>, request: Request) -> Response {
    if request.method() == Method::POST && request.uri().path().ends_with("/images/create") {
        let image = url::form_urlencoded::parse(request.uri().query().unwrap().as_bytes())
            .find(|(key, _)| key == "fromImage")
            .unwrap()
            .1
            .into_owned();
        state.pulls.lock().unwrap().push(image);
        return if state.reject {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"message":"registry transport refused"})),
            )
                .into_response()
        } else {
            Response::new(Body::from("{\"status\":\"Downloaded\"}\n"))
        };
    }
    let path = percent_encoding::percent_decode_str(request.uri().path())
        .decode_utf8()
        .unwrap();
    if path.contains("mirror.example") && !state.pulls.lock().unwrap().is_empty() {
        let digests: Vec<String> = state
            .digest
            .iter()
            .map(|digest| format!("mirror.example/library/alpine@{digest}"))
            .collect();
        return Json(json!({"Config":{},"RepoDigests":digests})).into_response();
    }
    (
        StatusCode::NOT_FOUND,
        Json(json!({"message":"No such image"})),
    )
        .into_response()
}

async fn engine(state: RegistryDocker) -> (Engine, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().fallback(serve).with_state(state))
            .await
            .unwrap();
    });
    (Engine::http_for_test(&url), server)
}

#[tokio::test]
async fn registry_pull_failure_never_attempts_the_original_registry() {
    let state = RegistryDocker {
        reject: true,
        ..Default::default()
    };
    let (engine, server) = engine(state.clone()).await;
    let image = format!("alpine:3@{DIGEST}");
    let error = engine
        .ensure_image(&image, PullPolicy::Missing, Some("mirror.example"))
        .await
        .unwrap_err();
    server.abort();
    assert!(matches!(error, EngineError::Pull { .. }));
    assert!(error.to_string().contains("registry transport refused"));
    assert_eq!(
        *state.pulls.lock().unwrap(),
        [mirrored(&image, "mirror.example").unwrap()]
    );
}

#[tokio::test]
async fn registry_pull_requires_a_matching_local_repo_digest() {
    for digest in [
        None,
        Some("sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_owned()),
        Some(DIGEST.to_owned()),
    ] {
        let state = RegistryDocker {
            digest: digest.clone(),
            ..Default::default()
        };
        let (engine, server) = engine(state).await;
        let image = format!("alpine:3@{DIGEST}");
        let result = engine
            .ensure_image(&image, PullPolicy::Missing, Some("mirror.example"))
            .await;
        server.abort();
        if digest.as_deref() == Some(DIGEST) {
            assert_eq!(result.unwrap(), mirrored(&image, "mirror.example").unwrap());
        } else {
            assert!(matches!(result, Err(EngineError::Pull { .. })));
        }
    }
}
