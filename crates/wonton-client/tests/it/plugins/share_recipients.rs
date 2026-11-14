//! 应用详情“分享给好友”的对象列表：读核心 `GET /_wonton/api/friends` 存着的好友（ADR-0051）。

use axum::http::Method;
use wonton_client::{CoreError, FriendsState};

use super::rig::{Canned, Rig};

const PATH: &str = "/_wonton/api/friends";

/// 核心名单的原样响应：没有页面状态，带黑名单和删过的人。
fn core_list() -> serde_json::Value {
    serde_json::json!({
        "friends": [{
            "user_id": "@alice:example.test",
            "display_name": "Alice",
            "role_id": "friend",
            "shared_plugins": ["app.navidrome"],
            "last_opened_share_at_ms": null
        }],
        "blocked_servers": [],
        "removed": [{"user_id": "@bob:example.test", "removed_at_ms": 1}],
        "can_block_federation": true,
        "owner_scope": true
    })
}

#[tokio::test]
async fn share_recipients_are_the_friends_the_core_keeps() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core
        .answer(Method::GET, PATH, Canned(200, core_list().to_string()));

    let recipients = rig.app.share_recipients().await.expect("读得懂核心的名单");
    assert_eq!(recipients.state, FriendsState::Ready);
    assert_eq!(recipients.friends.len(), 1);
    assert_eq!(recipients.friends[0].user_id, "@alice:example.test");
    assert_eq!(recipients.friends[0].shared_plugins, vec!["app.navidrome"]);
    assert_eq!(recipients.friends[0].role_id, "friend");
}

#[tokio::test]
async fn an_unreadable_list_is_an_upstream_failure_with_its_reason() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut broken = core_list();
    broken.as_object_mut().unwrap().remove("friends");
    rig.core
        .answer(Method::GET, PATH, Canned(200, broken.to_string()));

    let result = rig.app.share_recipients().await;
    assert!(
        matches!(&result, Err(CoreError::Upstream { reason }) if reason.contains("friends")),
        "{result:?}"
    );
}
