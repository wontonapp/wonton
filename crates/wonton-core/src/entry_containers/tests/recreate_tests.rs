//! 已有同名容器：陌生人、上一版 compose、换端口重建、重建失败、假 Docker 的地址冲突。

use serde_json::json;

use super::test_support::*;
use crate::entry_compose::HAPROXY_IMAGE;
use crate::plugin::fake_docker::FakeDocker;

/// 同名容器是别人建的：不动它，如实报冲突。
#[tokio::test]
async fn a_stranger_with_the_name_is_reported() {
    let rig = Rig::new().await;
    rig.docker
        .add_stranger_container("wonton-entry", json!({"Image": "other:1"}));
    let error = rig.reconcile(Some(&settings())).await.unwrap_err();
    assert_eq!(error.category(), "conflict");
    assert!(error.to_string().contains("wonton-entry"), "{error}");
    assert_eq!(rig.created("wonton-entry")["Image"], json!("other:1"));
}

/// 上一版由 compose 建的同名容器只带 compose 标签：认领、删掉、按现在的样子重建。
#[tokio::test]
async fn previous_compose_containers_are_adopted() {
    let rig = Rig::new().await;
    rig.docker.add_stranger_container(
        "wonton-entry",
        json!({"Image": HAPROXY_IMAGE, "Labels": {
            "com.docker.compose.project": PROJECT,
            "com.docker.compose.service": "entry",
        }}),
    );
    rig.reconcile(Some(&settings())).await.unwrap();
    let created = rig.created("wonton-entry");
    assert_eq!(created["Labels"]["wonton.core.component"], "entry");
    assert_eq!(rig.docker.creations("wonton-entry"), 1);
}

/// 换发布端口要重建：只管“镜像没变就什么都不做”会让改了端口一直是旧的。
/// 也是换法顺序的回归测试：旧容器在跑、占着 edge 固定地址时，先建新的再删旧的会被
/// fake Docker 的地址冲突模拟挡下（复审 451 在真 Docker 上实测的失败）。
#[tokio::test]
async fn a_changed_publish_port_recreates_the_connector() {
    let rig = Rig::new().await;
    let containers = rig.containers.with_binding("127.0.0.1", "18443");
    containers.reconcile(Some(&settings())).await.unwrap();
    let containers = containers.with_binding("127.0.0.1", "18444");
    containers.reconcile(Some(&settings())).await.unwrap();
    assert_eq!(rig.docker.creations("wonton-connector"), 2);
    assert_eq!(
        rig.docker.created("wonton-connector").unwrap_or_default()["HostConfig"]["PortBindings"]["8443/tcp"],
        json!([{"HostIp": "127.0.0.1", "HostPort": "18444"}])
    );
}

/// 创建失败（端口被占、Docker 出错）时入口缺席且如实报错：换法先删旧再建新，创建失败后
/// 要等下一次对账（退避最长 10 分钟）才建回来；错误按原因报，不假装入口还在。
#[tokio::test]
async fn a_failed_recreate_reports_and_waits_for_retry() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    let containers = rig.containers.with_binding("127.0.0.1", "18443");
    rig.docker
        .fail_next_create("wonton-connector", "bind: address already in use");
    let error = containers.reconcile(Some(&settings())).await.unwrap_err();
    assert_eq!(error.category(), "conflict");
    // 旧容器已按换法删掉，新的没建成：入口缺席，等下一次对账。
    assert!(
        !rig.docker
            .containers()
            .contains(&"wonton-connector".to_owned())
    );
    // 下一次对账建回来（fail_next_create 已在失败那次被消费）。
    containers.reconcile(Some(&settings())).await.unwrap();
    assert_eq!(
        rig.docker.created("wonton-connector").unwrap_or_default()["HostConfig"]["PortBindings"]["8443/tcp"],
        json!([{"HostIp": "127.0.0.1", "HostPort": "18443"}])
    );
}

/// fake Docker 要模拟真 Docker 的地址冲突：同一网络里运行中的容器占了固定 IP，第二个容器的 start
/// 报 Address already in use（复审 451 在真 Docker 29.7.2 上实测的原话）。
#[tokio::test]
async fn fake_docker_rejects_a_conflicting_fixed_ip_start() {
    let (docker, engine) = FakeDocker::start().await;
    docker.add_stranger_container("holder", json!({"Image": "img:1"}));
    docker.add_endpoint(
        "shared",
        "holder",
        json!({"IPAMConfig": {"IPv4Address": "10.0.0.9"}}),
    );
    docker.add_stranger_container("newcomer", json!({"Image": "img:1"}));
    docker.add_endpoint(
        "shared",
        "newcomer",
        json!({"IPAMConfig": {"IPv4Address": "10.0.0.9"}}),
    );
    let error = engine
        .start_container("newcomer")
        .await
        .expect_err("固定地址被运行中的容器占着");
    assert!(
        error.to_string().contains("Address already in use"),
        "{error}"
    );
    // 持有者停了，地址让出来，新容器起得来。
    docker.set_state(
        "holder",
        json!({"Status": "exited", "Running": false, "ExitCode": 0}),
    );
    engine.start_container("newcomer").await.unwrap();
}
