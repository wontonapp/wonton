//! 失败路径：重载失败跨轮重试、写下去后的容器动作失败、拉不到镜像不动在跑的容器。

use super::test_support::*;

/// 重载失败又碰上容器重建：失败那轮不能把证书记成已加载，下一轮要再试。
#[tokio::test]
async fn a_failed_reload_with_a_recreated_container_is_retried() {
    let mut rig = Rig::new().await;
    publish_cert(&rig, &["home.example.test", "*.home.example.test"]);
    app_domain(&rig, "home.example.test");
    rig.reconcile(None).await.unwrap();
    // 第一条连接拒（带启动日志），第二条起成功。
    let (addr, attempts) = failing_once_master().await;
    rig.containers.reload_addr = addr;
    rig.docker.remove("wonton-connector");
    publish_cert(&rig, &["home.example.test", "*.home.example.test"]);
    let error = rig.reconcile(None).await.unwrap_err();
    assert_eq!(error.category(), "upstream_error");
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    rig.reconcile(None).await.unwrap();
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "重载失败后的下一轮要再试"
    );
}

/// 只有配置变（证书没变）的重载失败，也要在下一轮重试：失败不能算恢复。
#[tokio::test]
async fn a_failed_config_reload_is_retried() {
    let mut rig = Rig::new().await;
    publish_cert(&rig, &["example.test", "*.example.test"]);
    app_domain(&rig, "example.test");
    rig.reconcile(None).await.unwrap();
    let (addr, attempts) = failing_once_master().await;
    rig.containers.reload_addr = addr;
    rig.docker.remove("wonton-connector");
    // 证书覆盖新名字，配置按新名字写；证书本身没换。
    app_domain(&rig, "home.example.test");
    rig.reconcile(None).await.unwrap_err();
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    rig.reconcile(None).await.unwrap();
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "配置变化的重载失败后下一轮要再试"
    );
}

/// 同上，但这一轮没有容器重建：失败的重试和“有没有 started”无关。
#[tokio::test]
async fn a_failed_config_reload_is_retried_without_a_restart() {
    let mut rig = Rig::new().await;
    publish_cert(&rig, &["example.test", "*.example.test"]);
    app_domain(&rig, "example.test");
    rig.reconcile(None).await.unwrap();
    let (addr, attempts) = failing_once_master().await;
    rig.containers.reload_addr = addr;
    app_domain(&rig, "home.example.test");
    rig.reconcile(None).await.unwrap_err();
    rig.reconcile(None).await.unwrap();
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "配置变化的重载失败也跨轮重试"
    );
}

/// 配置写下去之后、容器动作失败：这笔账不能丢，下一轮要让入口读新配置。
#[tokio::test]
async fn a_write_then_failed_container_action_is_retried() {
    let rig = Rig::new().await;
    publish_cert(&rig, &["example.test", "*.example.test"]);
    app_domain(&rig, "example.test");
    rig.reconcile(None).await.unwrap();
    let reloads = rig.reloads();
    app_domain(&rig, "home.example.test");
    rig.docker
        .fail_inspect("wonton-entry", Some("inspect boom"));
    rig.reconcile(None).await.unwrap_err();
    rig.docker.fail_inspect("wonton-entry", None);
    rig.reconcile(None).await.unwrap();
    assert_eq!(
        rig.reloads(),
        reloads + 1,
        "配置已经写下去，下一轮必须让入口读"
    );
}

/// 拉不到镜像时不动正在跑的容器，错误归 source_unavailable。
#[tokio::test]
async fn a_failed_pull_keeps_the_running_container() {
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.docker.fail_pulls(Some("toomanyrequests"));
    let error = rig.reconcile(Some(&settings())).await.unwrap_err();
    assert_eq!(error.category(), "source_unavailable");
    assert!(rig.docker.containers().contains(&"wonton-entry".to_owned()));
    assert!(
        rig.docker
            .containers()
            .contains(&"wonton-connector".to_owned())
    );
}
