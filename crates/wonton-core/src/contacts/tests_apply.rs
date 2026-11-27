//! 名单变更的应用回执（#646）：运行时真的换上了才算 applied，没换上按原因如实回；记录已经落下，
//! 失败也不撤联系人行。跨机事件和后台查邀请这两条没有人等着的路也记结果。

use std::sync::Arc;
use std::time::Duration;

use wonton_protocol::{ContactApply, ContactApplyFailure, ContactError};

use super::tests::{Net, Server, ask, bot_request, person_request, state};
use super::*;

fn failed(reason: ContactApplyFailure, detail: &str) -> ContactApply {
    ContactApply::Failed {
        reason,
        detail: detail.into(),
    }
}

#[tokio::test]
async fn an_applied_change_is_reported_as_applied() {
    let net = Arc::new(Net::default());
    let home = Server::start(&net, "a.test");
    let alice = home.bot("alice", "alice");
    let bob = home.bot("bob", "bob");
    ask(&home, &alice, bot_request("@hermes-bob:a.test"))
        .await
        .unwrap();

    let rows = home
        .contacts
        .approve(&bob, "bob", "@hermes-alice:a.test")
        .await
        .unwrap();
    assert_eq!(rows[0].state, ContactState::Active);
    // 两边都进了运行中的名单，两边都回报。
    assert_eq!(
        home.contacts.last_apply(&alice),
        Some(ContactApply::Applied)
    );
    assert_eq!(home.contacts.last_apply(&bob), Some(ContactApply::Applied));
    assert_eq!(
        home.contacts.allowed_users("@hermes-bob:a.test"),
        ["@hermes-alice:a.test"]
    );
}

#[tokio::test]
async fn a_stopped_plugin_is_reported_as_not_ready_and_the_record_stands() {
    let net = Arc::new(Net::default());
    let home = Server::start(&net, "a.test");
    let alice = home.bot("alice", "alice");
    let bob = home.bot("bob", "bob");
    ask(&home, &alice, bot_request("@hermes-bob:a.test"))
        .await
        .unwrap();
    home.effects.fail(
        "bob",
        failed(ContactApplyFailure::PluginNotReady, "插件停着"),
    );

    // 记录成了，调用不失败；回执说名单没换上。
    let rows = home
        .contacts
        .approve(&bob, "bob", "@hermes-alice:a.test")
        .await
        .unwrap();
    assert_eq!(rows[0].state, ContactState::Active);
    assert_eq!(
        state(&home, &alice, "@hermes-bob:a.test"),
        Some(ContactState::Active)
    );
    assert_eq!(
        home.contacts.last_apply(&bob),
        Some(failed(ContactApplyFailure::PluginNotReady, "插件停着"))
    );
    assert_eq!(
        home.contacts.last_apply(&alice),
        Some(ContactApply::Applied)
    );
    assert_eq!(
        home.contacts.allowed_users("@hermes-bob:a.test"),
        ["@hermes-alice:a.test"],
        "记录是准的，下次重建照记录来"
    );
}

#[tokio::test]
async fn failures_are_kept_by_reason_until_the_next_change() {
    let net = Arc::new(Net::default());
    let home = Server::start(&net, "a.test");
    let alice = home.bot("alice", "alice");
    let bob = home.bot("bob", "bob");
    ask(&home, &alice, bot_request("@hermes-bob:a.test"))
        .await
        .unwrap();
    home.effects
        .fail("bob", failed(ContactApplyFailure::Upstream, "上游 500"));
    home.contacts
        .approve(&bob, "bob", "@hermes-alice:a.test")
        .await
        .unwrap();
    assert_eq!(
        home.contacts.last_apply(&bob),
        Some(failed(ContactApplyFailure::Upstream, "上游 500"))
    );

    // 删掉也是一次变更：这次也没换上，最新的原因换成权限。
    home.effects.fail(
        "bob",
        failed(ContactApplyFailure::PermissionDenied, "没给 matrix.bot"),
    );
    home.contacts
        .remove(&bob, "bob", "@hermes-alice:a.test")
        .await
        .unwrap();
    assert_eq!(
        home.contacts.last_apply(&bob),
        Some(failed(
            ContactApplyFailure::PermissionDenied,
            "没给 matrix.bot"
        ))
    );

    // 再变一次，这次换上了：最新的结果是 applied。
    ask(&home, &alice, bot_request("@hermes-bob:a.test"))
        .await
        .unwrap();
    home.contacts
        .approve(&bob, "bob", "@hermes-alice:a.test")
        .await
        .unwrap();
    assert_eq!(home.contacts.last_apply(&bob), Some(ContactApply::Applied));
}

#[tokio::test]
async fn invalid_input_fails_the_call_without_a_receipt() {
    let net = Arc::new(Net::default());
    let home = Server::start(&net, "a.test");
    let alice = home.bot("alice", "alice");
    let err = ask(&home, &alice, bot_request("@bad")).await.unwrap_err();
    assert_eq!(err.kind, ContactError::InvalidUserId);
    assert_eq!(home.contacts.last_apply(&alice), None);
    assert!(home.rebuilt().is_empty());
}

#[tokio::test]
async fn the_list_carries_the_latest_result_and_a_restart_forgets_it() {
    let net = Arc::new(Net::default());
    let home = Server::start(&net, "a.test");
    let alice = home.bot("alice", "alice");
    home.effects.fail(
        "alice",
        failed(ContactApplyFailure::TransportUnavailable, "Docker 连不上"),
    );
    home.contacts
        .request(&alice, "alice", &person_request("@sam:a.test"))
        .await
        .unwrap();
    // 人接受邀请由后台查出来，没有人等这次应用的结果。
    home.invites
        .states
        .lock()
        .unwrap()
        .insert("!dm-@sam:a.test".into(), InviteeState::Joined);
    home.contacts.check_people().await;
    assert_eq!(
        state(&home, &alice, "@sam:a.test"),
        Some(ContactState::Active)
    );
    assert_eq!(
        home.contacts.last_apply(&alice),
        Some(failed(
            ContactApplyFailure::TransportUnavailable,
            "Docker 连不上"
        ))
    );
    home.contacts.list(&alice, "alice").await.unwrap();
    assert_eq!(
        home.contacts.last_apply(&alice),
        Some(failed(
            ContactApplyFailure::TransportUnavailable,
            "Docker 连不上"
        ))
    );

    let home = home.restart(&net, "a.test");
    assert_eq!(home.contacts.last_apply(&alice), None, "运行时事实不跨重启");
}

#[tokio::test]
async fn a_change_from_the_peer_core_is_recorded_without_a_caller() {
    let net = Arc::new(Net::default());
    let a = Server::start(&net, "a.test");
    let b = Server::start(&net, "b.test");
    let alice = a.bot("alice", "alice");
    let bob = b.bot("bob", "bob");
    ask(&b, &bob, bot_request("@hermes-alice:a.test"))
        .await
        .unwrap();
    net.deliver_all().await;
    assert_eq!(
        state(&a, &alice, "@hermes-bob:b.test"),
        Some(ContactState::Incoming)
    );
    assert_eq!(a.contacts.last_apply(&alice), None, "等人处理时名单没变");

    // A 同意后回结果，B 那边的 Bot 名单跟着变；这条路上没有人等响应，结果记在状态里。
    b.effects
        .fail("bob", failed(ContactApplyFailure::Upstream, "上游 500"));
    a.contacts
        .approve(&alice, "alice", "@hermes-bob:b.test")
        .await
        .unwrap();
    net.deliver_all().await;
    assert_eq!(
        state(&b, &bob, "@hermes-alice:a.test"),
        Some(ContactState::Active)
    );
    assert_eq!(
        b.contacts.last_apply(&bob),
        Some(failed(ContactApplyFailure::Upstream, "上游 500"))
    );
}

/// 客户端等不及断了（30 秒超时）：重建照样跑完、写回执，只是这次响应看不见。
#[tokio::test]
async fn a_disconnected_request_does_not_cancel_the_rebuild() {
    let net = Arc::new(Net::default());
    let home = Server::start(&net, "a.test");
    let alice = home.bot("alice", "alice");
    let bob = home.bot("bob", "bob");
    ask(&home, &alice, bot_request("@hermes-bob:a.test"))
        .await
        .unwrap();
    let hold = home.effects.hold("bob");
    let mut approve = Box::pin(home.contacts.approve(&bob, "bob", "@hermes-alice:a.test"));
    tokio::select! {
        _ = &mut approve => panic!("还没等到任务停住"),
        _ = hold.reached() => {}
    }
    // 真的把请求 future 丢掉：等于客户端超时断开。
    drop(approve);
    assert_eq!(home.contacts.last_apply(&bob), None);

    hold.release();
    tokio::time::timeout(Duration::from_secs(1), async {
        while home.contacts.last_apply(&bob).is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("后台任务要写完回执");
    assert_eq!(home.contacts.last_apply(&bob), Some(ContactApply::Applied));
    assert_eq!(
        home.contacts.allowed_users("@hermes-bob:a.test"),
        ["@hermes-alice:a.test"],
        "记录和名单都照常落下"
    );
}
