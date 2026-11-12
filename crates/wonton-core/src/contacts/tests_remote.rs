//! 跨机 Bot 联系人（ADR-0050）的行为测试：两台服务器经内存里的通道交换 `app.wonton.contact` 事件。

use std::sync::Arc;

use wonton_protocol::{ContactError, ContactEvent, ContactState, PeerStatus};

use super::PeerReach;
use super::tests::{Net, Server, ask, bot_request, state};

const ALICE: &str = "@hermes-alice:a.test";
const BOB: &str = "@hermes-bob:b.test";

fn two() -> (Arc<Net>, Server, Server) {
    let net = Arc::new(Net::default());
    let a = Server::start(&net, "a.test");
    let b = Server::start(&net, "b.test");
    a.bot("alice", "alice");
    b.bot("bob", "bob");
    (net, a, b)
}

fn alice() -> wonton_protocol::BotRef {
    wonton_protocol::BotRef {
        plugin_id: "ai.nous.hermes".into(),
        bot: "alice".into(),
    }
}

fn bob() -> wonton_protocol::BotRef {
    wonton_protocol::BotRef {
        plugin_id: "ai.nous.hermes".into(),
        bot: "bob".into(),
    }
}

#[tokio::test]
async fn a_bot_on_a_paired_server_becomes_a_contact_when_its_keeper_approves() {
    let (net, a, b) = two();
    let rows = ask(&a, &alice(), bot_request(BOB)).await.unwrap();
    assert_eq!(rows[0].state, ContactState::Pending);
    let remote = rows[0].remote.clone().unwrap();
    assert_eq!(remote.server_name, "b.test");
    assert_eq!(remote.peer_status, Some(PeerStatus::Connected));
    net.deliver_all().await;

    let pending = b.contacts.pending_for("bob");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].contact.user_id, ALICE);
    assert_eq!(
        pending[0].contact.display_name.as_deref(),
        Some("Hermes alice")
    );
    assert_eq!(
        pending[0].contact.keeper_display_name.as_deref(),
        Some("ALICE")
    );
    let pushed = b.effects.pushed.lock().unwrap().clone();
    assert_eq!(pushed.len(), 1, "新请求推送给 Bob");
    assert_eq!(pushed[0].0, "bob");

    b.contacts.approve(&bob(), "bob", ALICE).await.unwrap();
    assert_eq!(state(&b, &bob(), ALICE), Some(ContactState::Active));
    assert_eq!(b.rebuilt(), ["bob"]);
    assert_eq!(
        state(&a, &alice(), BOB),
        Some(ContactState::Pending),
        "结果还没送到"
    );
    net.deliver_all().await;
    assert_eq!(state(&a, &alice(), BOB), Some(ContactState::Active));
    assert_eq!(a.rebuilt(), ["alice"]);
    assert_eq!(
        a.row(&alice(), BOB).unwrap().keeper_display_name.as_deref(),
        Some("BOB"),
        "对方核心自报的归属人"
    );
    assert_eq!(a.contacts.allowed_users(ALICE), [BOB]);
    assert_eq!(b.contacts.allowed_users(BOB), [ALICE]);

    // 删掉时告诉对方，两边都不再处理对方的话。
    a.contacts.remove(&alice(), "alice", BOB).await.unwrap();
    net.deliver_all().await;
    assert_eq!(state(&b, &bob(), ALICE), None);
    assert!(b.contacts.allowed_users(BOB).is_empty());
    assert_eq!(b.rebuilt(), ["bob", "bob"]);
}

#[tokio::test]
async fn a_rejected_or_unknown_remote_bot_is_reported_to_the_requester() {
    let (net, a, b) = two();
    ask(&a, &alice(), bot_request(BOB)).await.unwrap();
    ask(&a, &alice(), bot_request("@hermes-nobody:b.test"))
        .await
        .unwrap();
    net.deliver_all().await;
    assert_eq!(
        state(&a, &alice(), "@hermes-nobody:b.test"),
        Some(ContactState::NotFound)
    );
    b.contacts.reject(&bob(), "bob", ALICE).await.unwrap();
    assert_eq!(state(&b, &bob(), ALICE), None);
    net.deliver_all().await;
    assert_eq!(state(&a, &alice(), BOB), Some(ContactState::Declined));
    assert!(a.rebuilt().is_empty());
}

#[tokio::test]
async fn an_unreachable_peer_fails_the_request_and_nothing_is_recorded() {
    let (net, a, b) = two();
    net.set_reach("b.test", PeerReach::Paired(PeerStatus::Offline));
    let failed = ask(&a, &alice(), bot_request(BOB)).await.unwrap_err();
    assert_eq!(failed.kind, ContactError::PeerUnreachable);
    assert!(net.queued().is_empty());

    net.set_reach("b.test", PeerReach::Paired(PeerStatus::Checking));
    a.fail_sends(Some(ContactError::PeerUnreachable));
    let failed = ask(&a, &alice(), bot_request(BOB)).await.unwrap_err();
    assert_eq!(failed.kind, ContactError::PeerUnreachable);
    assert_eq!(state(&a, &alice(), BOB), None);

    // 配对移除了：不是“连不上”，是“没配对”。
    net.set_reach("b.test", PeerReach::NotPaired);
    let failed = ask(&a, &alice(), bot_request(BOB)).await.unwrap_err();
    assert_eq!(failed.kind, ContactError::NotPaired);
    assert!(b.contacts.pending_for("bob").is_empty());
}

#[tokio::test]
async fn approving_fails_honestly_when_the_answer_cannot_be_sent() {
    let (net, a, b) = two();
    ask(&a, &alice(), bot_request(BOB)).await.unwrap();
    net.deliver_all().await;
    b.fail_sends(Some(ContactError::Upstream));
    let failed = b.contacts.approve(&bob(), "bob", ALICE).await.unwrap_err();
    assert_eq!(failed.kind, ContactError::Upstream);
    assert_eq!(
        state(&b, &bob(), ALICE),
        Some(ContactState::Incoming),
        "还在等处理"
    );
    assert!(b.rebuilt().is_empty());
}

#[tokio::test]
async fn both_sides_asking_at_once_become_contacts_without_approval() {
    let (net, a, b) = two();
    ask(&a, &alice(), bot_request(BOB)).await.unwrap();
    ask(&b, &bob(), bot_request(ALICE)).await.unwrap();
    net.deliver_all().await;
    assert_eq!(state(&a, &alice(), BOB), Some(ContactState::Active));
    assert_eq!(state(&b, &bob(), ALICE), Some(ContactState::Active));
    assert!(a.contacts.pending_for("alice").is_empty());
    assert!(b.contacts.pending_for("bob").is_empty());
}

#[tokio::test]
async fn a_late_answer_to_a_cancelled_request_does_not_count() {
    let (net, a, b) = two();
    ask(&a, &alice(), bot_request(BOB)).await.unwrap();
    net.deliver_all().await;
    // Bob 拒绝的回答还在路上，Alice 已经取消、又请求了一次。
    b.contacts.reject(&bob(), "bob", ALICE).await.unwrap();
    a.contacts.remove(&alice(), "alice", BOB).await.unwrap();
    ask(&a, &alice(), bot_request(BOB)).await.unwrap();
    net.deliver_all().await;
    assert_eq!(state(&a, &alice(), BOB), Some(ContactState::Pending));
    assert_eq!(state(&b, &bob(), ALICE), Some(ContactState::Incoming));
}

#[tokio::test]
async fn events_whose_user_ids_do_not_match_the_sender_are_ignored() {
    let (_net, a, _b) = two();
    let forged = ContactEvent::Request {
        id: "x".into(),
        from: "@hermes-mallory:c.test".into(),
        to: ALICE.into(),
        from_display_name: None,
        keeper_display_name: None,
    };
    a.contacts.receive("b.test", forged).await;
    let elsewhere = ContactEvent::Request {
        id: "y".into(),
        from: BOB.into(),
        to: "@hermes-alice:c.test".into(),
        from_display_name: None,
        keeper_display_name: None,
    };
    a.contacts.receive("b.test", elsewhere).await;
    assert!(a.contacts.pending_for("alice").is_empty());
    assert!(a.contacts.view(&alice()).is_empty());
}
