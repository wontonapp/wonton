use super::*;
use crate::chat::{ChatChannels, ChatLink, target::TimelineOpening};
use wonton_protocol::{PushMethod, TimelineTarget};

fn rig() -> (Arc<ChatLink>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Handle::current();
    let (_, method) = watch::channel(PushMethod::default());
    let home = crate::home::HomeLink::new(
        runtime.clone(),
        crate::sharing::hub::bind(runtime.clone()).unwrap(),
        crate::home::Storage::open(dir.path(), dir.path()).unwrap(),
        crate::home::Network::DirectOnly,
        None,
        method.clone(),
    )
    .unwrap();
    let plugins =
        crate::plugins::PluginsLink::new(runtime.clone(), home.clone(), dir.path(), None, false);
    #[cfg(not(feature = "test-hooks"))]
    let card_success: crate::chat::CardSuccess = Arc::new(|_| {});
    #[cfg(feature = "test-hooks")]
    let card_success: crate::chat::CardSuccess = Arc::new(|_| Box::pin(async {}));
    let chat = ChatLink::new(
        runtime,
        home,
        "test-room-lifecycle",
        dir.path(),
        card_success,
        plugins,
        ChatChannels {
            view: None,
            confirmations: None,
            method,
            lock_screen: watch::channel(crate::lock_screen::Hidden::default()).1,
            app_lock: crate::app_lock::AppLockLink::new(dir.path()),
            session_out: watch::Sender::new(None),
            external: false,
        },
    )
    .unwrap();
    (chat, dir)
}

fn begin(chat: &Arc<ChatLink>, room: &str, thread: Option<&str>) -> TimelineOpening {
    chat.begin_timeline(&TimelineTarget {
        room_id: room.to_owned(),
        thread_root: thread.map(str::to_owned),
    })
    .unwrap()
}

#[tokio::test]
async fn forgetting_the_session_rejects_a_pending_room_open() {
    let (chat, _dir) = rig();
    let pending = begin(&chat, "!room:example.com", None);
    chat.forget_current().await;
    let room_view = watch::channel(None).1;
    assert!(chat.open_timeline(pending, room_view).await.is_err());
}

#[tokio::test]
async fn a_timeline_does_not_receive_another_rooms_panel() {
    let (chat, _dir) = rig();
    let room: OwnedRoomId = "!room:example.com".parse().unwrap();
    let other: OwnedRoomId = "!other:example.com".parse().unwrap();
    let selected = wonton_protocol::TabRef::Url {
        url: "https://example.com/report".to_owned(),
    };
    let panel = |room_id| {
        Some(Arc::new(crate::pages::RoomPanelView {
            room_id,
            panel: Vec::new(),
            selected: Some(selected.clone()),
            blocked: true,
            widgets: Vec::new(),
        }))
    };
    let (sender, mut updates) = mpsc::unbounded_channel();
    let room_view = watch::Sender::new(panel(other));
    let outbox = Outbox::new(
        sender,
        Arc::default(),
        room_view.subscribe(),
        chat.view.subscribe(),
        room.clone(),
        watch::Sender::new(Arc::new(Vec::new())).subscribe(),
    );
    outbox
        .send(
            &Vector::new(),
            Vec::new(),
            BackPagination::Idle,
            TimelineViewport::Bottom,
            None,
            false,
        )
        .unwrap();
    let update = updates.try_recv().unwrap();
    assert_eq!(update.panel_selected, None);
    assert!(!update.entry_blocked);

    room_view.send_replace(panel(room));
    outbox
        .send(
            &Vector::new(),
            Vec::new(),
            BackPagination::Idle,
            TimelineViewport::Bottom,
            None,
            false,
        )
        .unwrap();
    let update = updates.try_recv().unwrap();
    assert_eq!(update.panel_selected, Some(selected));
    assert!(update.entry_blocked);
}

fn own_bot_ready() -> wonton_protocol::OwnPrivateBotView {
    wonton_protocol::OwnPrivateBotView::Ready {
        bot: wonton_protocol::BotRef {
            plugin_id: "app.whoami".into(),
            bot: "mia-bot".into(),
        },
        user_id: "@mia-bot:test".into(),
        home_room: None,
        room_problem: None,
    }
}

#[tokio::test]
async fn late_own_private_bot_result_is_dropped_after_session_change() {
    let chat = rig().0;
    let mut watch = chat.bots.view.subscribe();
    let stale = crate::chat::lock(&chat.bots.own_epoch).current();
    chat.bots.set_session(None);
    watch.borrow_and_update();
    chat.bots
        .publish_own_private_bot(stale, Ok(own_bot_ready()));
    assert_eq!(watch.borrow_and_update().own_private_bot, None);

    let current = crate::chat::lock(&chat.bots.own_epoch).current();
    chat.bots
        .publish_own_private_bot(current, Ok(own_bot_ready()));
    assert!(matches!(
        watch.borrow_and_update().own_private_bot,
        Some(wonton_protocol::OwnPrivateBotView::Ready { .. })
    ));
}

#[tokio::test]
async fn task_that_already_upgraded_cannot_publish_after_the_link_is_dropped() {
    let chat = rig().0;
    let bots = chat.bots.clone();
    let mut watch = bots.view.subscribe();
    let stale = crate::chat::lock(&bots.own_epoch).current();
    // Drop 只退役、不清共享快照：没有新作用域写入时槽保持初始 None，迟到结果仍被拒。
    drop(chat);
    bots.publish_own_private_bot(stale, Ok(own_bot_ready()));
    assert_eq!(watch.borrow_and_update().own_private_bot, None);
}

#[tokio::test]
async fn reused_channel_ignores_a_result_of_the_previous_server() {
    let chat = rig().0;
    let bots = chat.bots.clone();
    let sender = bots.view.clone();
    let mut watch = sender.subscribe();
    let stale = crate::chat::lock(&bots.own_epoch).current();
    // 换服务器时 scope 先撤旧资格、复用同一 sender 写入新作用域的初始值，旧链接随后才析构。
    let mut new_scope = watch.borrow().clone();
    chat.abandon_own_private_bot();
    let retired = crate::chat::lock(&bots.own_epoch).current();
    new_scope.own_private_bot = Some(new_scope_bot());
    new_scope.own_private_bot_problem = Some(own_bot_problem());
    sender.send_replace(new_scope);
    // 旧代的结果被拒，新作用域写下的身份与问题原样保留，不被旧 Ready 覆盖。
    bots.publish_own_private_bot(stale, Ok(own_bot_ready()));
    assert_new_scope_preserved(&mut watch, "after stale publish");
    // 新作用域已写入自己的身份；旧链接还没 Drop，它的前台／换会话都不能再清新槽。
    chat.bots.foregrounded();
    chat.bots.set_session(None);
    assert_new_scope_preserved(&mut watch, "after old foreground and set_session");
    assert_eq!(crate::chat::lock(&bots.own_epoch).current(), retired);
    // Drop 也只退役、不清共享快照：新作用域的身份保持。
    drop(chat);
    bots.publish_own_private_bot(stale, Ok(own_bot_ready()));
    assert_new_scope_preserved(&mut watch, "after Drop and late publish");
}

#[tokio::test]
async fn a_foreground_paused_across_the_server_switch_cannot_touch_the_new_scope() {
    let chat = rig().0;
    let bots = chat.bots.clone();
    let sender = bots.view.clone();
    let mut watch = sender.subscribe();
    // 旧作用域先完成一次真实捕获。会话是 None：Bots::new 的正常初始状态，也是切服时旧链接的常态。
    chat.bots.foregrounded();
    let captured = crate::chat::lock(&bots.own_epoch).current();

    // F4 的交错：旧链接的前台调用在捕获之后、写槽之前受控暂停（旧 Arc 还在手上），
    // abandon 和新作用域写初始值都在暂停期间完成。
    let (gate_tx, gate_rx) = tokio::sync::oneshot::channel();
    let paused_bots = bots.clone();
    let paused = tokio::spawn(async move {
        gate_rx.await.unwrap();
        paused_bots.foregrounded();
    });
    chat.abandon_own_private_bot();
    let mut new_scope = watch.borrow().clone();
    new_scope.own_private_bot = Some(new_scope_bot());
    new_scope.own_private_bot_problem = Some(own_bot_problem());
    sender.send_replace(new_scope);

    // 放行同一次前台调用，让它在交接后跑完：不得再发号，也不得清新作用域的双槽。
    gate_tx.send(()).unwrap();
    paused.await.unwrap();
    assert_new_scope_preserved(&mut watch, "paused case 1");

    // 注销路径的 set_session(None) 在交接后跑完。它同步执行，发号与清槽之间没有可挂起的点，
    // 用捕获的旧代把同一条真实清槽路径继续走完，也不得清。
    bots.set_session(None);
    bots.refresh_own_private_bot(None, captured);
    assert_new_scope_preserved(&mut watch, "paused case 2");

    // 旧链接 Drop 在后：迟到的结果同样被拒。
    drop(chat);
    bots.publish_own_private_bot(captured, Ok(own_bot_ready()));
    assert_new_scope_preserved(&mut watch, "paused case 3");
}

#[tokio::test]
async fn set_session_none_paused_after_capture_preserves_the_new_scope() {
    let (chat, _dir) = rig();
    let bots = chat.bots.clone();
    let mut watch = bots.view.subscribe();
    let (captured_tx, captured_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    crate::chat::lock(&bots.own_epoch).before_refresh = Some(Box::new(move || {
        captured_tx.send(()).unwrap();
        resume_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("old None refresh was not released");
    }));
    let old_bots = bots.clone();
    let pending = tokio::task::spawn_blocking(move || old_bots.set_session(None));
    tokio::time::timeout(std::time::Duration::from_secs(10), captured_rx)
        .await
        .expect("set_session did not capture its None refresh")
        .unwrap();

    // 同一次 set_session 已替换会话并配发代号，但它的 None 清槽尚未检查发布资格。
    chat.abandon_own_private_bot();
    let mut new_scope = watch.borrow().clone();
    new_scope.own_private_bot = Some(new_scope_bot());
    new_scope.own_private_bot_problem = Some(own_bot_problem());
    bots.view.send_replace(new_scope);
    resume_tx.send(()).unwrap();
    pending.await.unwrap();
    assert_new_scope_preserved(&mut watch, "after captured None refresh completed");

    bots.foregrounded();
    assert_new_scope_preserved(&mut watch, "after retired foregrounded");
    bots.set_session(None);
    assert_new_scope_preserved(&mut watch, "after retired set_session");
    drop(chat);
    assert_new_scope_preserved(&mut watch, "after old link Drop");
}

fn own_bot_problem() -> wonton_protocol::OwnBotProblem {
    wonton_protocol::OwnBotProblem {
        category: wonton_protocol::OwnBotFailure::TransportUnavailable,
        kind: None,
        status: None,
        detail: "传输不可用".into(),
    }
}

/// 新作用域自己的身份，取值与 [`own_bot_ready`] 不同：保留断言才能区分"新身份还在"和"旧结果写进来"。
fn new_scope_bot() -> wonton_protocol::OwnPrivateBotView {
    wonton_protocol::OwnPrivateBotView::Ready {
        bot: wonton_protocol::BotRef {
            plugin_id: "app.other".into(),
            bot: "other-bot".into(),
        },
        user_id: "@other-bot:new".into(),
        home_room: Some("!other:new".into()),
        room_problem: None,
    }
}

/// 断言新作用域的本人身份与问题字段都原样保留：看 watch 的实际内容，不用预期 None 或代数数字掩盖。
fn assert_new_scope_preserved(
    watch: &mut tokio::sync::watch::Receiver<wonton_protocol::ChatViewModel>,
    stage: &str,
) {
    let view = watch.borrow_and_update();
    assert_eq!(
        view.own_private_bot,
        Some(new_scope_bot()),
        "stage: {stage}"
    );
    assert_eq!(view.own_private_bot_problem, Some(own_bot_problem()));
}

#[tokio::test]
async fn foregrounded_reissues_the_own_bot_read_and_clears_a_stale_problem() {
    let chat = rig().0;
    let mut watch = chat.bots.view.subscribe();
    let initial_epoch = crate::chat::lock(&chat.bots.own_epoch).current();
    chat.bots
        .publish_own_private_bot(initial_epoch, Err(own_bot_problem()));
    assert!(watch.borrow_and_update().own_private_bot_problem.is_some());

    chat.bots.foregrounded();
    let retry_epoch = crate::chat::lock(&chat.bots.own_epoch).current();
    assert!(retry_epoch > initial_epoch);
    chat.bots
        .publish_own_private_bot(retry_epoch, Ok(own_bot_ready()));
    let view = watch.borrow_and_update();
    assert!(view.own_private_bot_problem.is_none());
    assert!(matches!(
        view.own_private_bot,
        Some(wonton_protocol::OwnPrivateBotView::Ready { .. })
    ));

    let current = crate::chat::lock(&chat.bots.own_epoch).current();
    assert_eq!(current, retry_epoch);
}

#[tokio::test]
async fn a_result_left_over_from_the_previous_foreground_cannot_write_back() {
    let chat = rig().0;
    let mut watch = chat.bots.view.subscribe();
    let before = crate::chat::lock(&chat.bots.own_epoch).current();
    chat.bots.foregrounded();
    chat.bots
        .publish_own_private_bot(before, Ok(own_bot_ready()));
    let view = watch.borrow_and_update();
    assert!(view.own_private_bot.is_none());
    assert!(view.own_private_bot_problem.is_none());
}

#[tokio::test]
async fn foregrounded_without_a_session_clears_both_own_bot_slots() {
    let chat = rig().0;
    let mut watch = chat.bots.view.subscribe();
    let epoch = crate::chat::lock(&chat.bots.own_epoch).current();
    chat.bots
        .publish_own_private_bot(epoch, Ok(own_bot_ready()));
    assert!(watch.borrow_and_update().own_private_bot.is_some());

    chat.bots.foregrounded();
    let view = watch.borrow_and_update();
    assert!(view.own_private_bot.is_none());
    assert!(view.own_private_bot_problem.is_none());
}
