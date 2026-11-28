//! 同步的生命周期：网络断了自己重启、第一次房间列表到达前不算在同步、身份被拒就停。

use std::time::Duration;

use futures_util::StreamExt;

use super::{Chat, receives_in_order, send};
use crate::support::{Relay, TestServer, unique};
use wonton_matrix::{MatrixSession, StoreConfig, SyncStatus};

/// 同步因网络错误停下后自己重启，还在同一个会话上：打开着的房间收到断开期间别人发的消息。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn sync_restarts_by_itself_and_the_open_room_catches_up() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    let keeping = async {
        chat.alice.keep_syncing().await;
    };
    let scenario = async {
        chat.relay.cut();
        wait_for_sync(&chat.alice, |status| {
            matches!(status, SyncStatus::Failed(_))
        })
        .await;
        send(&chat.bob_room, "断开时对方发的").await;
        chat.relay.restore();
        receives_in_order(&chat.alice_room, &["断开时对方发的"]).await;
    };
    tokio::select! {
        () = keeping => panic!("同步的重启任务提前结束了"),
        () = scenario => {}
    }
    assert_eq!(
        chat.alice.sync_status().next().await,
        Some(SyncStatus::Running)
    );
}

/// 房间列表第一次同步完成前不算在同步：新设备这时还没有房间，界面不能当成“没有聊天”。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn sync_is_not_running_before_the_first_room_list_arrives() {
    let server = TestServer::start().await;
    let relay = Relay::start(&server.url).await;
    let alice = server
        .login_via(
            relay.url(),
            &unique("alice"),
            "Alice 的手机",
            StoreConfig::InMemory,
        )
        .await;
    relay.cut();
    let statuses = alice.sync_status();
    let seen = async {
        let mut statuses = std::pin::pin!(statuses);
        let mut seen = Vec::new();
        while let Some(status) = statuses.next().await {
            let failed = matches!(status, SyncStatus::Failed(_));
            seen.push(status);
            if failed {
                return seen;
            }
        }
        seen
    };
    let (seen, started) = tokio::join!(seen, alice.start_sync());
    assert!(started.is_err(), "连不上服务器时同步启动不了");
    assert!(seen.contains(&SyncStatus::Starting), "{seen:?}");
    assert!(!seen.contains(&SyncStatus::Running), "{seen:?}");

    relay.restore();
    alice.start_sync().await.expect("连接恢复后开始同步");
    assert_eq!(alice.sync_status().next().await, Some(SyncStatus::Running));
}

/// 服务器拒绝身份时重启也没用，不重试。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn sync_stays_stopped_when_the_server_rejects_the_device() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    chat.relay.reject();
    tokio::time::timeout(Duration::from_secs(60), chat.alice.keep_syncing())
        .await
        .expect("服务器拒绝身份后还在重启同步");
}
async fn wait_for_sync(session: &MatrixSession, mut done: impl FnMut(&SyncStatus) -> bool) {
    let mut statuses = std::pin::pin!(session.sync_status());
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(status) = statuses.next().await {
            if done(&status) {
                return;
            }
        }
    })
    .await
    .expect("等同步状态超时");
}
