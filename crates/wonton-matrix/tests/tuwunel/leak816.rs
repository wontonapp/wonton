//! #816：同步过的 Client 必须能整体释放，进程内重建会话时旧实例不再滞留。
//!
//! 登录后 `sync_once` 一次，丢掉会话和手里的全部强引用。SDK 内部若有任务还拿着客户端
//! （补丁见 `vendor/README.md` 的 LatestEvents 一节），弱引用就取不回 `None`。

use std::time::Duration;

use matrix_sdk::config::SyncSettings;
use wonton_matrix::StoreConfig;
use wonton_matrix::testkit::{TestServer, unique};

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn synced_client_is_released() {
    let server = TestServer::start().await;
    let session = server
        .login_via(&server.url, &unique("leak816"), "g", StoreConfig::InMemory)
        .await;
    let client = session.client().clone();
    let weak = client.downgrade();

    client
        .sync_once(SyncSettings::default().timeout(Duration::from_secs(1)))
        .await
        .expect("同步一次");

    drop(client);
    drop(session);

    // 释放发生在 drop 之后的任务收尾，给一个有上限的等待窗口。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while weak.get().is_some() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        weak.get().is_none(),
        "同步过的 Client 在 5 秒后仍被 SDK 内部持有"
    );
}
