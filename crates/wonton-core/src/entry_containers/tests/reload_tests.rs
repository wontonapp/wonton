//! 向入口主进程发 reload：失败报启动日志，忙、空回、半截、连上不回都重试。

use std::sync::atomic::Ordering;
use std::time::Duration;

use super::test_support::*;

/// 证书续期后向入口 HAProxy 的主进程发 `reload`：按组件标签找，不经 Docker 的 kill 接口。
#[tokio::test]
async fn reload_asks_the_entry_to_soft_reload() {
    let (addr, seen) = master_cli("Success=1\n").await;
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.containers.reload(&addr).await.unwrap();
    let seen = tokio::time::timeout(Duration::from_secs(5), seen)
        .await
        .expect("没有连管理端口")
        .unwrap();
    assert_eq!(seen, "reload\n");
    assert!(
        rig.writes_for("/kill").is_empty(),
        "{:?}",
        rig.docker.writes()
    );
}

/// 新配置没起来：`Success=0` 后面跟着启动日志，原样报出来。
#[tokio::test]
async fn reload_reports_the_startup_logs_of_a_failed_reload() {
    let (addr, _seen) =
        master_cli("Success=0\n--\n[ALERT] (49) : config : missing timeouts for proxy 'public'.\n")
            .await;
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    let error = rig.containers.reload(&addr).await.unwrap_err();
    let text = error.to_string();
    assert!(text.contains("missing timeouts"), "{text}");
}

/// 容器刚起来：主进程还在初始装载，第一次回“还在重载”，隔一会儿重发后成功。
/// 忙一次要等 15 秒，拨快时钟别真睡。
#[tokio::test(start_paused = true)]
async fn reload_retries_while_another_reload_is_in_progress() {
    let (addr, hits) = master_sequence(vec![
        "Success=0\n--\nAnother reload is still in progress.\n",
        "Success=1\n",
    ])
    .await;
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.containers.reload(&addr).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

/// 主进程接了就断、还没开始回应：也算可重试。
#[tokio::test]
async fn reload_retries_a_connection_closed_without_a_reply() {
    let (addr, hits) = master_sequence(vec!["", "Success=1\n"]).await;
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.containers.reload(&addr).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

/// 连上却一个字都不回：不能吃光整段预算，隔一小段换连接重发。
#[tokio::test]
async fn reload_retries_a_silent_connection() {
    let (addr, hits) = master_silent_then(vec!["Success=1\n"]).await;
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.containers.reload(&addr).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

/// 主进程只回了半截 `Success=0\n` 就断：不是拒绝，换条连接重发。
#[tokio::test]
async fn reload_retries_a_truncated_rejection() {
    let (addr, hits) = master_sequence(vec!["Success=0\n", "Success=1\n"]).await;
    let rig = Rig::new().await;
    rig.reconcile(Some(&settings())).await.unwrap();
    rig.containers.reload(&addr).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}
