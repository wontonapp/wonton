//! 会话释放后没发出去的消息还在本机，下次建好会话就接着发。

use super::{Chat, failed, receives_in_order, send};
use crate::support::TempStore;
use wonton_matrix::{MatrixSession, SessionConfig};

/// 会话释放时还没发出去的存在本机，下次建好会话就接着发，不用打开那个房间，也不用等同步。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn messages_left_unsent_go_out_once_the_session_is_back() {
    let store = TempStore::create();
    let chat = Chat::open(store.config()).await;
    chat.relay.cut();
    send(&chat.alice_room, "重建前没发出去的").await;
    failed(&chat.alice_room, "重建前没发出去的", true).await;

    let Chat {
        alice_room,
        bob_room,
        alice,
        relay,
        server,
        ..
    } = chat;
    let saved = alice.saved().expect("登录状态");
    alice.stop_sync().await;
    drop(alice_room);
    drop(alice);

    relay.restore();
    let config = SessionConfig {
        homeserver_url: relay.url().to_owned(),
        ..server.config("Alice 的手机", store.config())
    };
    let _alice = MatrixSession::restore(&config, saved)
        .await
        .expect("恢复会话");
    receives_in_order(&bob_room, &["重建前没发出去的"]).await;
}
