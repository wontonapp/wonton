//! 同步过的 Client 必须能整体释放：进程内重建会话时旧实例不再滞留（#67，归档 #816）。
//!
//! 登录后至少 `sync_once` 一次，SDK 会建 `LatestEvents` 并让它的监听任务持有 `SendQueue`
//! （补丁见 `vendor/README.md` 的 LatestEvents 一节）。任务把客户端按住，就永远等不到自己该退出的通道关闭。
//! 丢掉会话和手里的全部强引用，弱引用必须取回 `None`。

use std::time::Duration;

use matrix_sdk::{WeakClient, config::SyncSettings};
use wonton_matrix::testkit::{TempStore, TestServer, eventually, unique};
use wonton_matrix::{MatrixSession, Recovery, StoreConfig, Verification};

/// 释放发生在 drop 之后的任务收尾，给一个有上限的等待窗口。
const RELEASE: Duration = Duration::from_secs(5);

/// 丢掉会话、停同步，等 SDK 内部把客户端也放掉。
async fn released(session: MatrixSession) -> bool {
    let weak = session.client().downgrade();
    session.stop_sync().await;
    drop(session);
    gone(&weak, RELEASE).await
}

async fn gone(weak: &WeakClient, window: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + window;
    while weak.get().is_some() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    weak.get().is_none()
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_synced_client_is_released() {
    let server = TestServer::start().await;
    let session = server
        .login_via(&server.url, &unique("leak"), "g", StoreConfig::InMemory)
        .await;
    session
        .client()
        .sync_once(SyncSettings::default().timeout(Duration::from_secs(1)))
        .await
        .expect("同步一次");

    assert!(
        released(session).await,
        "同步过的 Client 在 {RELEASE:?} 后仍被 SDK 内部持有"
    );
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_client_that_recovered_its_keys_is_released() {
    let server = TestServer::start().await;
    let user = unique("leak");
    let first = server
        .login_via(&server.url, &user, "第一台", StoreConfig::InMemory)
        .await;
    first.start_sync().await.expect("第一台同步");
    eventually("第一台已验证", first.encryption_states(), |state| {
        (state.verification == Verification::Verified).then_some(())
    })
    .await;
    // 恢复密钥要等备份上传完才返回。
    let key = first.enable_recovery().await.expect("开启恢复");
    first.stop_sync().await;
    drop(first);

    let session = server
        .login_via(&server.url, &user, "第二台", StoreConfig::InMemory)
        .await;
    session.start_sync().await.expect("第二台同步");
    eventually("待恢复", session.encryption_states(), |state| {
        (state.recovery == Recovery::Incomplete).then_some(())
    })
    .await;
    session.recover(&key).await.expect("输入恢复密钥");

    assert!(
        released(session).await,
        "恢复过密钥的 Client 在 {RELEASE:?} 后仍被 SDK 内部持有"
    );
}

/// 真实的进程内重建：旧会话在新会话打开同一个存储之前必须已经放掉，新会话才能接着用。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_client_is_released_before_a_rebuild_on_the_same_store() {
    let server = TestServer::start().await;
    let user = unique("leak");
    let store = TempStore::create();
    let old = server
        .login_via(&server.url, &user, "old", store.config())
        .await;
    old.start_sync().await.expect("旧会话同步");
    let weak = old.client().downgrade();
    let saved = old.saved().expect("登录状态");
    old.stop_sync().await;
    drop(old);
    assert!(
        gone(&weak, RELEASE).await,
        "旧会话丢掉引用 {RELEASE:?} 后还活着"
    );

    let new = MatrixSession::restore(&server.config("old", store.config()), saved)
        .await
        .expect("同一个存储上重建会话");
    new.start_sync().await.expect("新会话同步");
}
