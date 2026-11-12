//! 联系人行为测试：同机 Bot 之间的请求和审批、跨机经 `on_remote_*` 的三步握手、人的邀请和接受。
//! 配对名单和 Bot 名录用测试替身（共享的替身在 [`super::test_support`]）；Matrix 用真的假 Tuwunel
//! （`crate::matrix::fake`）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use wonton_protocol::{
    BotRef, ContactError, ContactEvent, ContactKind, ContactOutcome, ContactRequest, ContactState,
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
    let f = open_contacts(config, dir_path).await;
    // 生产里 main 起的房间补建监视器：配对变化后由字节序较小的一方建核心联系人房间。
    exchange::spawn(&f.contacts);
    f
}

/// 只建 Contacts，不起收件循环：直接驱动 `process_response` 的测试用它，循环会抢着写同步令牌。
async fn open_contacts(config: &MatrixConfig, dir_path: &std::path::Path) -> Fixture {
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
    // 修好后再发一轮（等退避窗口过去）：房间建成、邀请已发，但对方核心还没进房——“还没通”如实留着。
    fake.state().refuse_rooms = false;
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    super::exchange::drain(&f.contacts, "zz.example").await;
    let view = f.contacts.view(&a_bot());
    assert_eq!(
        view.contacts[0].remote.as_ref().unwrap().room_error,
        Some(ContactError::PeerUnreachable)
    );
    // 对方核心进了房：状态清掉，事件送出去。
    let room = f.contacts.peer_room("zz.example").unwrap();
    fake.state()
        .rooms
        .get_mut(&room)
        .unwrap()
        .members
        .insert("@wonton-core:zz.example".to_owned());
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

/// 第二轮验收：并发 commit 一条不丢（审查 #1）。
#[tokio::test]
async fn concurrent_commits_do_not_lose_updates() {
    let f = fixture().await;
    let mut handles = Vec::new();
    for i in 0..32 {
        let contacts = f.contacts.clone();
        handles.push(tokio::spawn(async move {
            contacts
                .commit(|stored| {
                    stored.enqueue(
                        "zz.example",
                        ContactEvent::Remove {
                            from: format!("@helper-{i}:zz.example"),
                            to: "@a:test".to_owned(),
                        },
                        format!("txn-{i}"),
                        i as u64,
                    );
                })
                .unwrap();
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }
    assert_eq!(f.contacts.stored_outbox().len(), 32, "入队一条都不能丢");
}

/// 第二轮验收：并发同意两个请求，两行都落成 Active（审查 #1）。
#[tokio::test]
async fn concurrent_approves_all_land() {
    let f = fixture().await;
    f.contacts
        .on_remote_request(
            &a_bot(),
            "@u1:zz.example",
            "zz.example",
            "req-1",
            None,
            None,
        )
        .unwrap();
    f.contacts
        .on_remote_request(
            &a_bot(),
            "@u2:zz.example",
            "zz.example",
            "req-2",
            None,
            None,
        )
        .unwrap();
    let (c1, c2) = (f.contacts.clone(), f.contacts.clone());
    let bot = a_bot();
    let u1 = "@u1:zz.example";
    let u2 = "@u2:zz.example";
    let (r1, r2) = tokio::join!(c1.approve(&bot, u1), c2.approve(&bot, u2),);
    r1.unwrap();
    r2.unwrap();
    assert_eq!(
        a_row(&f, "@u1:zz.example").unwrap().state,
        ContactState::Active
    );
    assert_eq!(
        a_row(&f, "@u2:zz.example").unwrap().state,
        ContactState::Active
    );
}

/// 第二轮验收：互发直接成立时，Active 行和给对方的同意回复在同一个事务里（审查 #3）。
#[tokio::test]
async fn mutual_establishment_persists_active_and_reply_together() {
    let f = fixture().await;
    f.peers.pair("zz.example");
    request_a(&f, "@helper:zz.example", ContactKind::Bot).await;
    let reply = f
        .contacts
        .on_remote_request(
            &a_bot(),
            "@helper:zz.example",
            "zz.example",
            "theirs-1",
            None,
            None,
        )
        .unwrap();
    assert!(reply.is_some(), "互发直接成立要回一条同意");
    assert_eq!(
        a_row(&f, "@helper:zz.example").unwrap().state,
        ContactState::Active
    );
    // 回复已经在 outbox 里（和 Active 同一个事务落盘），中间崩了也不会只有一半。
    let outbox = f.contacts.stored_outbox();
    assert!(outbox.iter().any(
        |entry| matches!(&entry.event, ContactEvent::Result { id, outcome: ContactOutcome::Approved, .. } if id == "theirs-1")
    ));
}

/// 第二轮验收：取消配对清掉那台服务器的 outbox（审查 #5）。
#[tokio::test]
async fn unpairing_drops_the_servers_outbox() {
    let f = fixture().await;
    f.peers.pair("zz.example");
    request_a(&f, "@helper:zz.example", ContactKind::Bot).await;
    assert!(
        !f.contacts.stored_outbox().is_empty(),
        "请求没送到（对方核心没进房），事件应该在 outbox 里"
    );
    f.peers.unpair("zz.example");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f.contacts.stored_outbox().is_empty() {
        assert!(Instant::now() < deadline, "取消配对后 outbox 没被清掉");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 一份带 peer 邀请的 /sync 响应（invite_state 的 stripped 形状）。
fn invite_sync_response(room: &str, inviter: &str, next_batch: &str) -> serde_json::Value {
    serde_json::json!({
        "next_batch": next_batch,
        "rooms": {"invite": {room: {"invite_state": {"events": [
            {"type": "m.room.create", "sender": inviter,
             "content": {"type": wonton_protocol::CONTACT_ROOM_TYPE}},
            {"type": "m.room.member", "sender": inviter,
             "content": {"membership": "invite"}},
        ]}}}}
    })
}

async fn poll_matrix(config: &MatrixConfig, dir_path: &std::path::Path) -> Arc<Matrix> {
    Arc::new(
        Matrix::connect(config, dir_path, crate::http::HttpClient::new())
            .await
            .unwrap(),
    )
}

/// 第二轮验收：邀请 join 失败时这批不推进令牌——增量 /sync 不会再给这条邀请，
/// 只有从旧令牌重来才看得见；重投后加入成功（审查 #2）。
#[tokio::test]
async fn a_failed_invite_join_defers_the_token() {
    let (fake, config) = crate::matrix::fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let f = open_contacts(&config, dir.path()).await;
    f.peers.pair("zz.example");
    // 假 Tuwunel 里造出这条邀请对应的房间：类型对、我们的核心在邀请名单里。
    {
        use std::collections::BTreeMap;
        use std::collections::BTreeSet;
        let mut state = fake.state();
        state.rooms.insert(
            "!room:zz.example".to_owned(),
            crate::matrix::fake::Room {
                members: BTreeSet::from(["@wonton-core:zz.example".to_owned()]),
                invited: BTreeSet::from(["@wonton-core:test".to_owned()]),
                left: BTreeSet::new(),
                encrypted: false,
                room_type: Some(wonton_protocol::CONTACT_ROOM_TYPE.to_owned()),
                creator: Some("@wonton-core:zz.example".to_owned()),
                invited_by: BTreeMap::from([(
                    "@wonton-core:test".to_owned(),
                    "@wonton-core:zz.example".to_owned(),
                )]),
            },
        );
    }
    let matrix = poll_matrix(&config, dir.path()).await;
    let response = invite_sync_response("!room:zz.example", "@wonton-core:zz.example", "s1");
    // join 被拒：整批作废，令牌不落盘。
    fake.state().refuse_join = true;
    let result = super::exchange::process_response(&f.contacts, &matrix, response.clone()).await;
    assert!(result.is_err(), "join 失败要作废整批");
    assert!(
        !dir.path().join("contacts-sync-token").exists(),
        "令牌不推进"
    );
    // 重投：join 成功，房间记下、令牌落盘。
    fake.state().refuse_join = false;
    let next = super::exchange::process_response(&f.contacts, &matrix, response)
        .await
        .unwrap();
    assert_eq!(next.as_deref(), Some("s1"));
    assert_eq!(
        f.contacts.peer_room("zz.example").as_deref(),
        Some("!room:zz.example")
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("contacts-sync-token")).unwrap(),
        "s1"
    );
}

/// 第二轮验收：同步令牌写盘失败时整批作废、令牌不推进（审查 #3）。
#[tokio::test]
async fn a_failed_token_write_defers_the_token() {
    let (_fake, config) = crate::matrix::fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let f = open_contacts(&config, dir.path()).await;
    f.peers.pair("zz.example");
    let matrix = poll_matrix(&config, dir.path()).await;
    let token = dir.path().join("contacts-sync-token");
    // 第一批成功：令牌 s1 落盘。
    let next = super::exchange::process_response(
        &f.contacts,
        &matrix,
        serde_json::json!({"next_batch": "s1"}),
    )
    .await
    .unwrap();
    assert_eq!(next.as_deref(), Some("s1"));
    // 令牌文件变成目录：写盘失败，整批作废。
    std::fs::remove_file(&token).unwrap();
    std::fs::create_dir(&token).unwrap();
    let result = super::exchange::process_response(
        &f.contacts,
        &matrix,
        serde_json::json!({"next_batch": "s2"}),
    )
    .await;
    assert!(result.is_err(), "令牌写不进去要作废整批");
    // 修好：重投同一批，推进。
    std::fs::remove_dir(&token).unwrap();
    let next = super::exchange::process_response(
        &f.contacts,
        &matrix,
        serde_json::json!({"next_batch": "s2"}),
    )
    .await
    .unwrap();
    assert_eq!(next.as_deref(), Some("s2"));
}
