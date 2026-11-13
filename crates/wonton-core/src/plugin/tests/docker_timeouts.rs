//! Docker 处理得慢时的时限和错误类别。

use std::time::Duration;

use bollard::models::ContainerCreateBody;
use serde_json::json;

use super::super::docker::{Engine, EngineError};
use super::super::fake_docker::{FakeDocker, Hold};
use super::super::install_error::InstallError;

/// 普通请求 1 秒就超时的客户端。
async fn impatient() -> (FakeDocker, Engine) {
    let (docker, engine) = FakeDocker::start().await;
    let engine = engine.with_request_limit_for_test(Duration::from_secs(1));
    (docker, engine)
}

/// 过了普通请求的时限才让 Docker 回答。
async fn answer_late(hold: Hold) {
    hold.reached().await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    hold.release();
}

#[tokio::test]
async fn stopping_and_removing_a_container_outlast_the_limit_of_ordinary_requests() {
    for call in ["POST /containers/x/stop", "DELETE /containers/x"] {
        let (docker, engine) = impatient().await;
        docker.add_container("x", "plugin");
        let hold = docker.hold(call);
        let (removed, ()) = tokio::join!(engine.remove_container("x"), answer_late(hold));
        removed.unwrap();
        assert!(docker.containers().is_empty(), "{call}");
    }
}

#[tokio::test]
async fn removing_a_network_or_a_volume_outlasts_the_limit_of_ordinary_requests() {
    let (docker, engine) = impatient().await;
    docker.add_network("n", "plugin", json!({}));
    docker.add_volume("v", "plugin");

    let hold = docker.hold("DELETE /networks/n");
    let (removed, ()) = tokio::join!(engine.remove_network("n"), answer_late(hold));
    removed.unwrap();
    assert!(docker.networks().is_empty());

    let hold = docker.hold("DELETE /volumes/v");
    let (removed, ()) = tokio::join!(engine.remove_volume("v"), answer_late(hold));
    removed.unwrap();
    assert!(docker.volumes().is_empty());
}

#[tokio::test]
async fn a_request_docker_does_not_answer_in_time_is_not_reported_as_docker_being_unavailable() {
    let (docker, engine) = impatient().await;
    docker.hold("POST /containers/create");
    let error = engine
        .create_container("x", ContainerCreateBody::default())
        .await
        .unwrap_err();
    assert!(matches!(error, EngineError::TimedOut(_)), "{error}");
    assert!(matches!(
        InstallError::from_create_engine_error(error),
        InstallError::DockerTimedOut(_)
    ));
}
