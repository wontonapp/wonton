//! 契约：应用详情的“分享给好友”按 `wonton_protocol::FriendsList` 读 `GET /_wonton/api/friends`。
//! 这里用核心真实的响应体去解析，名单的各种情况都要读得懂。

use axum::{body::Body, extract::Request};
use hyper::StatusCode;
use iroh::SecretKey;
use tower::ServiceExt;
use wonton_protocol::{FriendsList, ShareGrantUpdate, ShareRecipient};

use super::{Fixture, now_ms};
use crate::gateway::{TunnelPeer, router, tests::CORE, tests::Reply};
use crate::plugin::PluginId;

const ALICE: &str = "@alice:example.test";

async fn list(f: &Fixture) -> FriendsList {
    let owner = SecretKey::generate().public();
    f.gateway.tunnel.devices.add(owner, None).unwrap();
    let mut req = Request::builder()
        .uri("/_wonton/api/friends")
        .header("host", CORE)
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(TunnelPeer(owner));
    let reply = Reply::of(router(f.gateway.clone()).oneshot(req).await.unwrap()).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    serde_json::from_str(reply.text())
        .unwrap_or_else(|error| panic!("客户端读不懂核心的好友名单：{error}\n{}", reply.text()))
}

fn share(f: &Fixture, recipient: ShareRecipient) {
    f.sharing
        .shares
        .create(
            &ShareGrantUpdate {
                plugin_id: "app.whoami".into(),
                recipient,
                expires_at_ms: None,
                acknowledge_risk: true,
            },
            &f.h.manager
                .share_offer(&PluginId::parse("app.whoami").unwrap()),
        )
        .unwrap();
}

#[tokio::test]
async fn the_share_section_reads_the_core_friend_list_in_every_state() {
    let f = Fixture::new().await;

    // 有好友、还没分享。
    let first = list(&f).await;
    assert!(first.owner_scope);
    assert_eq!(first.friends.len(), 1);
    assert_eq!(first.friends[0].user_id, ALICE);
    assert!(first.friends[0].shared_plugins.is_empty());

    // 直接分享给好友。
    share(
        &f,
        ShareRecipient::Friend {
            user_id: ALICE.into(),
        },
    );
    let shared = list(&f).await;
    assert_eq!(shared.friends[0].role_id, "friend");
    assert_eq!(
        shared.friends[0].shared_plugins,
        vec!["app.whoami".to_owned()]
    );

    // 删掉好友：名单空了，删过的人留在 `removed`。
    f.sharing.friends.remove(ALICE, now_ms()).unwrap();
    let empty = list(&f).await;
    assert!(empty.friends.is_empty());
    assert_eq!(empty.removed.len(), 1);
    assert_eq!(empty.removed[0].user_id, ALICE);
}
