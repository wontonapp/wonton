//! 联系人行为测试：同机 Bot 之间的请求和审批、跨机经 `on_remote_*` 的三步握手、人的邀请和接受。
//! 配对名单和 Bot 名录用测试替身（共享的替身在 [`super::test_support`]）；Matrix 用真的假 Tuwunel
//! （`crate::matrix::fake`）。

use std::sync::Arc;

use wonton_protocol::{
    BotRef, ContactError, ContactKind, ContactOutcome, ContactRequest, ContactState,
};

use super::{
    Contacts, exchange,
    test_support::{FakeBots, FakePeers, FakePush},
};
use crate::config::MatrixConfig;
use crate::matrix::Matrix;

fn a_bot() -> BotRef {
    BotRef {
        plugin_id: "com.example.alpha".into(),
        bot: "helper".into(),
    }
}

fn b_bot() -> BotRef {
    BotRef {
        plugin_id: "com.example.alpha".into(),
        bot: "helper2".into(),
    }
}

struct Fixture {
    contacts: Arc<Contacts>,
    bots: Arc<FakeBots>,
    peers: Arc<FakePeers>,
    push: Arc<FakePush>,
    _dir: Option<tempfile::TempDir>,
}

async fn fixture() -> Fixture {
    let (_matrix, config) = crate::matrix::fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut f = open_fixture(&config, dir.path()).await;
    f._dir = Some(dir);
    f
}

/// 从已有目录重开：重启后行还在的测试用它，目录由调用方活着。
async fn reopen_fixture(config: &MatrixConfig, dir_path: &std::path::Path) -> Fixture {
    open_fixture(config, dir_path).await
}

async fn open_fixture(config: &MatrixConfig, dir_path: &std::path::Path) -> Fixture {
    // 行为测试的世界：本机是 test（假 Tuwunel 的 SERVER），远端是 zz.example。
    let peers = Arc::new(FakePeers::default());
    let bots = Arc::new(FakeBots::default());
    bots.add("@a:test", &a_bot(), "阿福", "主人甲", "owner-a");
    bots.add("@b:test", &b_bot(), "小二", "主人乙", "owner-b");
    let push = Arc::new(FakePush::default());
    let contacts = Contacts::open_with_backoff(
        dir_path.join("contacts.json"),
        peers.clone(),
        std::time::Duration::from_millis(20),
        std::time::Duration::from_millis(80),
    )
    .unwrap()
    .with_matrix(Arc::new(
        Matrix::connect(config, dir_path, crate::http::HttpClient::new())
            .await
            .unwrap(),
    ))
    .with_bots(bots.clone())
    .with_push(push.clone());
    // 生产里 main 起的房间补建监视器：配对变化后由字节序较小的一方建核心联系人房间。
    exchange::spawn(&contacts);
    Fixture {
        contacts,
        bots,
        peers,
        push,
        _dir: None,
    }
}

fn a_row(f: &Fixture, user: &str) -> Option<super::store::StoredContact> {
    f.contacts.stored_row(&a_bot(), user)
}

async fn request_a(f: &Fixture, user_id: &str, kind: ContactKind) {
    f.contacts
        .request(
            &a_bot(),
            &ContactRequest {
                user_id: user_id.to_owned(),
                kind,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn request_then_approve_activates_and_rebuilds() {
    let f = fixture().await;
    request_a(&f, "@b:test", ContactKind::Bot).await;
    // 发起方 Pending，接收方 Incoming 且收到推送。
    assert_eq!(a_row(&f, "@b:test").unwrap().state, ContactState::Pending);
    let incoming = f.contacts.stored_row(&b_bot(), "@a:test").unwrap();
    assert_eq!(incoming.state, ContactState::Incoming);
    // 名字不落行：同机的从本机 Bot 名录里现取，走的是名单视图。
    let seen = f
        .contacts
        .view(&b_bot())
        .contacts
        .into_iter()
        .find(|c| c.user_id == "@a:test")
        .unwrap();
    assert_eq!(seen.display_name.as_deref(), Some("阿福"));
    assert_eq!(seen.keeper_display_name.as_deref(), Some("主人甲"));
    assert_eq!(f.push.requests().len(), 1);
    // Incoming 不在名单里。
    assert!(f.contacts.allowed_users_extra(&b_bot()).is_empty());

    f.contacts.approve(&b_bot(), "@a:test").await.unwrap();
    let ours = a_row(&f, "@b:test").unwrap();
    assert_eq!(ours.state, ContactState::Active);
    // 名字走视图，不落行。
    let seen = f
        .contacts
        .view(&a_bot())
        .contacts
        .into_iter()
        .find(|c| c.user_id == "@b:test")
        .unwrap();
    assert_eq!(seen.display_name.as_deref(), Some("小二"));
    assert_eq!(seen.keeper_display_name.as_deref(), Some("主人乙"));
    let theirs = f.contacts.stored_row(&b_bot(), "@a:test").unwrap();
    assert_eq!(theirs.state, ContactState::Active);
    // 两边都进了对方的名单，两个 Bot 都重建。
    assert_eq!(f.contacts.allowed_users_extra(&a_bot()), ["@b:test"]);
    assert_eq!(f.contacts.allowed_users_extra(&b_bot()), ["@a:test"]);
    let rebuilds = f.bots.rebuilds();
    assert!(rebuilds.contains(&"@a:test".to_owned()));
    assert!(rebuilds.contains(&"@b:test".to_owned()));
    assert!(f.contacts.pending_requests().is_empty());
}

#[tokio::test]
async fn reject_marks_requester_declined() {
    let f = fixture().await;
    request_a(&f, "@b:test", ContactKind::Bot).await;
    f.contacts.reject(&b_bot(), "@a:test").await.unwrap();
    assert!(f.contacts.stored_row(&b_bot(), "@a:test").is_none());
    assert_eq!(a_row(&f, "@b:test").unwrap().state, ContactState::Declined);
}

#[tokio::test]
async fn delete_removes_both_rows() {
    let f = fixture().await;
    request_a(&f, "@b:test", ContactKind::Bot).await;
    f.contacts.approve(&b_bot(), "@a:test").await.unwrap();
    f.contacts.delete(&a_bot(), "@b:test").await.unwrap();
    assert!(a_row(&f, "@b:test").is_none());
    assert!(f.contacts.stored_row(&b_bot(), "@a:test").is_none());
    // 删除后两边名单都空了。
    assert!(f.contacts.allowed_users_extra(&a_bot()).is_empty());
    assert!(f.contacts.allowed_users_extra(&b_bot()).is_empty());
}

#[tokio::test]
async fn remote_request_pushes_and_wrong_result_ignored() {
    let f = fixture().await;
    f.peers.pair("zz.example");
    // 对方发来请求：落 Incoming、推送带对方自报的名字。
    assert_eq!(
        f.contacts
            .on_remote_request(
                &a_bot(),
                "@helper:zz.example",
                "zz.example",
                "req-1",
                Some("远端的福".into()),
                Some("远端主人".into())
            )
            .unwrap(),
        None
    );
    let row = a_row(&f, "@helper:zz.example").unwrap();
    assert_eq!(row.state, ContactState::Incoming);
    assert_eq!(row.remote.as_deref(), Some("zz.example"));
    assert_eq!(row.kind, ContactKind::Bot);
    let (keeper, request) = &f.push.requests()[0];
    assert_eq!(keeper.as_deref(), Some("owner-a"));
    assert_eq!(request.user_id, "@helper:zz.example");
    assert_eq!(request.display_name.as_deref(), Some("远端的福"));
    assert_eq!(request.keeper_display_name.as_deref(), Some("远端主人"));

    // 结果只翻发起方的 Pending 行；我们的 Incoming 行不被任何结果动到。
    f.contacts
        .on_remote_result(
            &a_bot(),
            "@helper:zz.example",
            "zz.example",
            "req-1",
            ContactOutcome::Approved,
            None,
        )
        .unwrap();
    let row = a_row(&f, "@helper:zz.example").unwrap();
    assert_eq!(row.state, ContactState::Incoming);

    f.contacts
        .on_remote_remove(&a_bot(), "@helper:zz.example", "zz.example")
        .unwrap();
    assert!(a_row(&f, "@helper:zz.example").is_none());
}

#[tokio::test]
async fn remote_result_rejected_declines() {
    let f = fixture().await;
    f.peers.pair("zz.example");
    request_a(&f, "@helper:zz.example", ContactKind::Bot).await;
    let id = a_row(&f, "@helper:zz.example").unwrap().request_id.unwrap();
    f.contacts
        .on_remote_result(
            &a_bot(),
            "@helper:zz.example",
            "zz.example",
            &id,
            ContactOutcome::Rejected,
            None,
        )
        .unwrap();
    // 行留着，落成“对方拒绝了”；Declined 不在名单里，不重建。
    assert_eq!(
        a_row(&f, "@helper:zz.example").unwrap().state,
        ContactState::Declined
    );
    assert!(f.bots.rebuilds().is_empty());
}

#[tokio::test]
async fn request_to_unpaired_server() {
    let f = fixture().await;
    let err = f
        .contacts
        .request(
            &a_bot(),
            &ContactRequest {
                user_id: "@helper:zz.example".into(),
                kind: ContactKind::Bot,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err, ContactError::NotPaired);
}

#[tokio::test]
async fn person_same_server_rejected() {
    let f = fixture().await;
    let err = f
        .contacts
        .request(
            &a_bot(),
            &ContactRequest {
                user_id: "@someone:test".into(),
                kind: ContactKind::Person,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err, ContactError::InvalidUserId);
}

#[tokio::test]
async fn person_invite_stores_room() {
    let f = fixture().await;
    f.peers.pair("zz.example");
    request_a(&f, "@person:zz.example", ContactKind::Person).await;
    let row = a_row(&f, "@person:zz.example").unwrap();
    assert_eq!(row.state, ContactState::Pending);
    assert_eq!(row.kind, ContactKind::Person);
    assert!(row.room.is_some(), "对人的邀请要建好私聊房间");
}

#[tokio::test]
async fn rows_survive_restart() {
    let (_matrix, config) = crate::matrix::fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    {
        let f = open_fixture(&config, dir.path()).await;
        f.peers.pair("zz.example");
        request_a(&f, "@helper:zz.example", ContactKind::Bot).await;
    }
    let f = reopen_fixture(&config, dir.path()).await;
    let after = a_row(&f, "@helper:zz.example").unwrap();
    assert_eq!(after.user_id, "@helper:zz.example");
    assert_eq!(after.state, ContactState::Pending);
}

#[tokio::test]
async fn approve_lands_first_and_the_event_waits_in_the_outbox() {
    let (_fake, config) = crate::matrix::fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    {
        let f = open_fixture(&config, dir.path()).await;
        f.peers.pair("zz.example");
        // 远端发来请求，归属人同意。
        f.contacts
            .on_remote_request(
                &b_bot(),
                "@helper:zz.example",
                "zz.example",
                "req-1",
                Some("远端的福".into()),
                None,
            )
            .unwrap();
        f.contacts
            .approve(&b_bot(), "@helper:zz.example")
            .await
            .unwrap();
        // 同意先落盘：行是 Active，结果事件在 outbox 里（对方核心还没进房，发不出去）。
        let row = f
            .contacts
            .stored_row(&b_bot(), "@helper:zz.example")
            .unwrap();
        assert_eq!(row.state, ContactState::Active);
        assert_eq!(f.contacts.stored_outbox().len(), 1);
        // 重启后同意还在（审查 #1）。
        let f = reopen_fixture(&config, dir.path()).await;
        let row = f
            .contacts
            .stored_row(&b_bot(), "@helper:zz.example")
            .unwrap();
        assert_eq!(row.state, ContactState::Active);
        assert_eq!(f.contacts.stored_outbox().len(), 1);
    }
    let f = reopen_fixture(&config, dir.path()).await;
    assert_eq!(f.contacts.stored_outbox().len(), 1);
}

#[tokio::test]
async fn outbox_delivers_once_the_peer_joins() {
    let (fake, config) = crate::matrix::fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let f = open_fixture(&config, dir.path()).await;
    f.peers.pair("zz.example");
    f.contacts
        .on_remote_request(
            &b_bot(),
            "@helper:zz.example",
            "zz.example",
            "req-1",
            Some("远端的福".into()),
            None,
        )
        .unwrap();
    f.contacts
        .approve(&b_bot(), "@helper:zz.example")
        .await
        .unwrap();
    assert_eq!(f.contacts.stored_outbox().len(), 1);
    // 对方核心进了房间（假 Tuwunel 里直接放进去），下一轮发送把事件送出去、outbox 清空。
    let room = f.contacts.peer_room("zz.example").expect("房间已建立");
    fake.state()
        .rooms
        .get_mut(&room)
        .unwrap()
        .members
        .insert("@wonton-core:zz.example".to_owned());
    super::exchange::drain(&f.contacts, "zz.example").await;
    assert!(f.contacts.stored_outbox().is_empty());
    let events = fake.state().events.clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].room, room);
    assert_eq!(events[0].content["type"], "result");
    assert_eq!(events[0].content["id"], "req-1");
    assert_eq!(events[0].content["outcome"], "approved");
}

#[tokio::test]
async fn repeated_failed_drains_do_not_pile_up_rooms() {
    let (fake, config) = crate::matrix::fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let f = open_fixture(&config, dir.path()).await;
    f.peers.pair("zz.example");
    // 对方一直不进房：退避让补建停下，房间只有一个，不会反复建房留孤儿。
    f.contacts
        .on_remote_request(
            &b_bot(),
            "@helper:zz.example",
            "zz.example",
            "req-1",
            None,
            None,
        )
        .unwrap();
    for _ in 0..3 {
        super::exchange::drain(&f.contacts, "zz.example").await;
    }
    let contact_rooms = fake
        .state()
        .rooms
        .values()
        .filter(|room| room.room_type == Some(wonton_protocol::CONTACT_ROOM_TYPE.to_owned()))
        .count();
    assert_eq!(contact_rooms, 1);
    assert!(f.contacts.peer_room("zz.example").is_some());
}

#[tokio::test]
async fn failed_room_establishment_shows_in_the_row() {
    let (fake, config) = crate::matrix::fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let f = open_fixture(&config, dir.path()).await;
    f.peers.pair("zz.example");
    // Tuwunel 拒绝建房：行如实显示失败（审查 #3）。
    fake.state().refuse_rooms = true;
    f.contacts
        .request(
            &a_bot(),
            &ContactRequest {
                user_id: "@helper:zz.example".to_owned(),
                kind: ContactKind::Bot,
            },
        )
        .await
        .unwrap();
    let view = f.contacts.view(&a_bot());
    let remote = view.contacts[0].remote.as_ref().unwrap();
    assert_eq!(remote.room_error, Some(ContactError::PeerUnreachable));
    // 修好后再发一轮：房间建成，失败清掉（等退避窗口过去）。
    fake.state().refuse_rooms = false;
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    super::exchange::drain(&f.contacts, "zz.example").await;
    let view = f.contacts.view(&a_bot());
    assert_eq!(view.contacts[0].remote.as_ref().unwrap().room_error, None);
}

#[tokio::test]
async fn deleting_an_incoming_remote_request_rejects_it() {
    let (fake, config) = crate::matrix::fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let f = open_fixture(&config, dir.path()).await;
    f.peers.pair("zz.example");
    f.contacts
        .on_remote_request(
            &b_bot(),
            "@helper:zz.example",
            "zz.example",
            "req-1",
            Some("远端的福".into()),
            None,
        )
        .unwrap();
    // 删一条进来的远端请求按拒绝算（ADR-0050），送回去的是 Rejected 不是 Remove（审查 #5）。
    f.contacts
        .delete(&b_bot(), "@helper:zz.example")
        .await
        .unwrap();
    assert!(
        f.contacts
            .stored_row(&b_bot(), "@helper:zz.example")
            .is_none()
    );
    let room = f.contacts.peer_room("zz.example").expect("房间已建立");
    fake.state()
        .rooms
        .get_mut(&room)
        .unwrap()
        .members
        .insert("@wonton-core:zz.example".to_owned());
    super::exchange::drain(&f.contacts, "zz.example").await;
    let events = fake.state().events.clone();
    assert_eq!(events[0].content["type"], "result");
    assert_eq!(events[0].content["outcome"], "rejected");
}

#[tokio::test]
async fn remote_remove_only_touches_the_addressed_bot() {
    let f = fixture().await;
    // 两个本机 Bot 都和同一个远端账号互为联系人。
    for bot in [&a_bot(), &b_bot()] {
        f.contacts.seed_row_for_test(
            bot,
            super::store::StoredContact {
                user_id: "@helper:zz.example".to_owned(),
                kind: ContactKind::Bot,
                state: ContactState::Active,
                remote: Some("zz.example".to_owned()),
                display_name: None,
                keeper_display_name: None,
                request_id: Some("req-1".to_owned()),
                room: None,
                updated_at_ms: 1,
            },
        );
    }
    // 对方只删了 a 的（事件里的 `to` 指着 a）：b 的行不能被牵连（审查 #2）。
    f.contacts
        .on_remote_remove(&a_bot(), "@helper:zz.example", "zz.example")
        .unwrap();
    assert!(a_row(&f, "@helper:zz.example").is_none());
    assert!(
        f.contacts
            .stored_row(&b_bot(), "@helper:zz.example")
            .is_some()
    );
}
