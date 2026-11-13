//! 有名字模式（ADR-0051）下加联系人的行为测试：开放联邦，加“人”不要求对方配对；
//! 私用名和 Bot 照旧要配对（ADR-0050）。

use std::sync::Arc;

use wonton_protocol::{ContactError, ContactState, PeerStatus};

use super::PeerReach;
use super::tests::{Net, Server, ask, bot_request, person_request, state};

#[tokio::test]
async fn an_open_server_adds_a_person_on_an_unpaired_server() {
    let net = Arc::new(Net::default());
    let home = Server::start_open(&net, "a.test");
    let alice_bot = home.bot("alice", "alice");

    // 对方是 matrix.org 这类没装云吞的服务器：邀请照发，先记“等对方接受”。
    let rows = ask(&home, &alice_bot, person_request("@xiaohong:matrix.org"))
        .await
        .unwrap();
    assert_eq!(rows[0].state, ContactState::Pending);
    assert_eq!(
        state(&home, &alice_bot, "@xiaohong:matrix.org"),
        Some(ContactState::Pending)
    );
    assert_eq!(home.invites.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_private_name_server_still_requires_pairing_for_a_person() {
    let net = Arc::new(Net::default());
    let home = Server::start(&net, "a.test");
    let alice_bot = home.bot("alice", "alice");

    let refused = ask(&home, &alice_bot, person_request("@xiaohong:matrix.org"))
        .await
        .unwrap_err();
    assert_eq!(refused.kind, ContactError::NotPaired);
    assert_eq!(state(&home, &alice_bot, "@xiaohong:matrix.org"), None);
    assert!(home.invites.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_bot_on_an_unpaired_server_still_requires_pairing() {
    let net = Arc::new(Net::default());
    let home = Server::start_open(&net, "a.test");
    let alice_bot = home.bot("alice", "alice");

    // app.wonton.contact 是配对专属通道，开放联邦也替代不了。
    let refused = ask(&home, &alice_bot, bot_request("@hermes-b:matrix.org"))
        .await
        .unwrap_err();
    assert_eq!(refused.kind, ContactError::NotPaired);
    assert_eq!(state(&home, &alice_bot, "@hermes-b:matrix.org"), None);
    assert!(net.queued().is_empty());
}

#[tokio::test]
async fn an_offline_paired_server_still_rejects_a_person_request() {
    // 有名字模式下配对过的目的地钉在 iroh 代理上：探活说下线，邀请也发不过去，照样快速失败。
    let net = Arc::new(Net::default());
    let home = Server::start_open(&net, "a.test");
    let alice_bot = home.bot("alice", "alice");
    net.set_reach("b.test", PeerReach::Paired(PeerStatus::Offline));

    let failed = ask(&home, &alice_bot, person_request("@xiaohong:b.test"))
        .await
        .unwrap_err();
    assert_eq!(failed.kind, ContactError::PeerUnreachable);
    assert_eq!(state(&home, &alice_bot, "@xiaohong:b.test"), None);
    assert!(home.invites.sent.lock().unwrap().is_empty());
}
