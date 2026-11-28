//! 卸载在请求断开后照样做完。

use std::sync::Arc;
use std::time::Duration;

use super::super::super::fake_docker::FakeDocker;
use super::super::super::names::container_name;
use super::super::super::{PluginId, PluginStatus};
use super::super::langs;
use super::{app, manager_with};

#[tokio::test]
async fn an_uninstall_finishes_after_its_request_goes_away() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir));
    m.install(&app("app.test"), &[], langs()).await.unwrap();
    let id = PluginId::parse("app.test").unwrap();
    let hold = docker.hold(&format!(
        "DELETE /containers/{}",
        container_name("app-test", "web")
    ));
    let request = tokio::spawn({
        let (m, id) = (m.clone(), id.clone());
        async move {
            m.uninstall_detached(id, true, Vec::new(), true, langs())
                .await
        }
    });
    hold.reached().await;
    // 请求断开时 hyper 丢弃处理它的 future。
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());

    // 卸载还在进行：再来一次如实回答，不另起一次。
    let err = m
        .uninstall_detached(id.clone(), true, Vec::new(), true, langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "conflict");
    assert_eq!(m.list()[0].status, PluginStatus::Uninstalling);

    hold.release();
    let mut changes = m.changes();
    tokio::time::timeout(
        Duration::from_secs(5),
        changes.wait_for(|_| m.list().is_empty()),
    )
    .await
    .expect("请求断开后卸载照样做完")
    .unwrap();
    assert!(docker.containers().is_empty(), "{:?}", docker.containers());
    assert!(docker.networks().is_empty(), "{:?}", docker.networks());
}
