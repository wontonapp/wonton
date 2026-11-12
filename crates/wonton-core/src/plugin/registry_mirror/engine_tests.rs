use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::json;

use crate::plugin::compose::PullPolicy;
use crate::plugin::docker::{Engine, EngineError};

const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const OTHER_DIGEST: &str =
    "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

/// 假的 Docker 守护进程：记下按哪个引用拉过，能按站拒拉、按站登记错摘要；只有拉过的引用才在本地。
#[derive(Clone, Default)]
struct RegistryDocker {
    pulls: Arc<Mutex<Vec<String>>>,
    rejecting: Arc<Mutex<HashSet<String>>>,
    wrong_digest: Arc<Mutex<HashSet<String>>>,
    digests: Arc<Mutex<HashMap<String, String>>>,
    /// 每个引用还能成功 inspect 的次数，用完就报 500：模拟 Docker inspect 出错。
    inspect_ok_left: Arc<Mutex<HashMap<String, usize>>>,
}

impl RegistryDocker {
    fn rejecting(stations: &[&str]) -> Self {
        Self {
            rejecting: Arc::new(Mutex::new(stations.iter().map(|s| s.to_string()).collect())),
            ..Self::default()
        }
    }

    fn wrong_digest(stations: &[&str]) -> Self {
        Self {
            wrong_digest: Arc::new(Mutex::new(stations.iter().map(|s| s.to_string()).collect())),
            ..Self::default()
        }
    }

    /// 让 `reference` 的第一次 inspect 成功、之后报 500。
    fn flaky_inspect(reference: &str) -> Self {
        Self {
            inspect_ok_left: Arc::new(Mutex::new(
                [(reference.to_owned(), 1)].into_iter().collect(),
            )),
            ..Self::default()
        }
    }

    fn station_of(reference: &str) -> &str {
        reference
            .split_once('/')
            .map_or(reference, |(station, _)| station)
    }

    fn pulls(&self) -> Vec<String> {
        self.pulls.lock().unwrap().clone()
    }
}

async fn serve(State(state): State<RegistryDocker>, request: Request) -> Response {
    if request.method() == Method::POST && request.uri().path().ends_with("/images/create") {
        let image = url::form_urlencoded::parse(request.uri().query().unwrap().as_bytes())
            .find(|(key, _)| key == "fromImage")
            .unwrap()
            .1
            .into_owned();
        state.pulls.lock().unwrap().push(image.clone());
        let station = RegistryDocker::station_of(&image).to_owned();
        if state.rejecting.lock().unwrap().contains(&station) {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"message":"registry transport refused"})),
            )
                .into_response();
        }
        let digest = if state.wrong_digest.lock().unwrap().contains(&station) {
            OTHER_DIGEST.to_owned()
        } else {
            image.rsplit_once('@').unwrap().1.to_owned()
        };
        state
            .digests
            .lock()
            .unwrap()
            .insert(image, digest.to_owned());
        return Response::new(Body::from("{\"status\":\"Downloaded\"}\n"));
    }
    let path = percent_encoding::percent_decode_str(request.uri().path())
        .decode_utf8()
        .unwrap();
    let name = path
        .strip_prefix("/images/")
        .and_then(|rest| rest.strip_suffix("/json"))
        .unwrap_or("");
    if let Some(remaining) = state.inspect_ok_left.lock().unwrap().get_mut(name) {
        if *remaining == 0 {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"message":"daemon inspect failed"})),
            )
                .into_response();
        }
        *remaining -= 1;
    }
    if let Some(digest) = state.digests.lock().unwrap().get(name) {
        let repository = name
            .split_once('@')
            .map_or(name, |(repository, _)| repository);
        return Json(json!({"Config":{},"RepoDigests":[format!("{repository}@{digest}")]}))
            .into_response();
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

fn stations(names: &[&str], repository: &str) -> Vec<String> {
    names
        .iter()
        .map(|name| format!("{name}/{repository}@{DIGEST}"))
        .collect()
}

#[tokio::test]
async fn registry_pull_failure_never_attempts_the_original_registry() {
    let state = RegistryDocker::rejecting(&["mirror.example"]);
    let (engine, server) = engine(state.clone()).await;
    let image = format!("alpine:3@{DIGEST}");
    let error = engine
        .ensure_image(
            &image,
            PullPolicy::Missing,
            &stations(&["mirror.example"], "library/alpine"),
        )
        .await
        .unwrap_err();
    server.abort();
    assert!(matches!(error, EngineError::Pull { .. }));
    assert!(error.to_string().contains("registry transport refused"));
    assert_eq!(
        state.pulls(),
        stations(&["mirror.example"], "library/alpine")
    );
}

/// 第一个站拒拉时换下一个，拉下来的镜像用第二个站的引用。
#[tokio::test]
async fn the_next_station_is_used_when_the_first_fails() {
    let state = RegistryDocker::rejecting(&["first.example"]);
    let (engine, server) = engine(state.clone()).await;
    let image = format!("ghcr.io/org/app@{}", DIGEST);
    let accelerated = stations(&["first.example", "second.example"], "org/app");
    let result = engine
        .ensure_image(&image, PullPolicy::Missing, &accelerated)
        .await;
    server.abort();
    assert_eq!(result.unwrap(), accelerated[1]);
    assert_eq!(state.pulls(), accelerated);
}

/// 加速站给了别的内容：换下一个，不算拉成功。
#[tokio::test]
async fn a_wrong_digest_falls_through_to_the_next_station() {
    let state = RegistryDocker::wrong_digest(&["first.example"]);
    let (engine, server) = engine(state.clone()).await;
    let image = format!("quay.io/org/app@{}", DIGEST);
    let accelerated = stations(&["first.example", "second.example"], "org/app");
    let result = engine
        .ensure_image(&image, PullPolicy::Missing, &accelerated)
        .await;
    server.abort();
    assert_eq!(result.unwrap(), accelerated[1]);
    assert_eq!(state.pulls(), accelerated);
}

/// 都拉不下来时如实报错：带上仓库和每个站各自的失败。
#[tokio::test]
async fn every_station_failing_reports_the_repository_and_each_station() {
    let state = RegistryDocker::rejecting(&["first.example", "second.example"]);
    let (engine, server) = engine(state.clone()).await;
    let image = format!("immich-app/immich-server@{}", DIGEST);
    let accelerated = stations(
        &["first.example", "second.example"],
        "immich-app/immich-server",
    );
    let error = engine
        .ensure_image(&image, PullPolicy::Missing, &accelerated)
        .await
        .unwrap_err();
    server.abort();
    let EngineError::Pull {
        image: reported,
        message,
    } = &error
    else {
        panic!("{error}");
    };
    assert_eq!(reported, &image);
    for expected in [
        image.as_str(),
        "first.example",
        "second.example",
        "registry transport refused",
    ] {
        assert!(message.contains(expected), "{message}");
    }
    assert_eq!(state.pulls(), accelerated);
}

/// 已经在本地、摘要又对得上的加速镜像不再拉。
#[tokio::test]
async fn a_present_verified_accelerated_image_is_reused() {
    let state = RegistryDocker::default();
    {
        let accelerated = stations(&["mirror.example"], "library/alpine");
        state
            .digests
            .lock()
            .unwrap()
            .insert(accelerated[0].clone(), DIGEST.to_owned());
    }
    let (engine, server) = engine(state.clone()).await;
    let image = format!("alpine:3@{}", DIGEST);
    let result = engine
        .ensure_image(
            &image,
            PullPolicy::Never,
            &stations(&["mirror.example"], "library/alpine"),
        )
        .await;
    server.abort();
    assert_eq!(
        result.unwrap(),
        stations(&["mirror.example"], "library/alpine")[0]
    );
    assert!(state.pulls().is_empty());
}

/// 加速站里的同引用摘要对不上、又没别的站：报错，不当成能用。
#[tokio::test]
async fn a_present_image_with_the_wrong_digest_is_not_reused() {
    let state = RegistryDocker::default();
    {
        let accelerated = stations(&["mirror.example"], "library/alpine");
        state
            .digests
            .lock()
            .unwrap()
            .insert(accelerated[0].clone(), OTHER_DIGEST.to_owned());
    }
    let (engine, server) = engine(state).await;
    let image = format!("alpine:3@{}", DIGEST);
    let error = engine
        .ensure_image(
            &image,
            PullPolicy::Never,
            &stations(&["mirror.example"], "library/alpine"),
        )
        .await
        .unwrap_err();
    server.abort();
    assert!(matches!(error, EngineError::NotFound(_)), "{error}");
}

/// 没有加速引用时照旧直连，拉失败报的也是直连的错。
#[tokio::test]
async fn without_stations_the_original_registry_is_used() {
    let state = RegistryDocker::default();
    let (engine, server) = engine(state.clone()).await;
    let image = format!("quay.io/org/app@{}", DIGEST);
    let result = engine.ensure_image(&image, PullPolicy::Missing, &[]).await;
    server.abort();
    assert_eq!(result.unwrap(), image);
    assert_eq!(state.pulls(), [image]);
}

/// Docker inspect 出错时如实报错，不当成摘要匹配继续用本机镜像。
#[tokio::test]
async fn an_inspect_error_is_not_treated_as_a_digest_match() {
    let image = format!("alpine:3@{DIGEST}");
    let state = RegistryDocker::flaky_inspect(&image);
    state
        .digests
        .lock()
        .unwrap()
        .insert(image.clone(), DIGEST.to_owned());
    let (engine, server) = engine(state).await;
    let error = engine
        .ensure_image(&image, PullPolicy::Never, &[])
        .await
        .unwrap_err();
    server.abort();
    assert!(
        matches!(error, EngineError::Rejected { .. }),
        "inspect 出错该按上游出错上报：{error}"
    );
    assert!(
        error.to_string().contains("daemon inspect failed"),
        "{error}"
    );
}
