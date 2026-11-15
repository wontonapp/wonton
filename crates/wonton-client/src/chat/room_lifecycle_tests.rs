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
    let stale = *crate::chat::lock(&chat.bots.own_epoch);
    chat.bots.set_session(None);
    watch.borrow_and_update();
    chat.bots
        .publish_own_private_bot(stale, Ok(own_bot_ready()));
    assert_eq!(watch.borrow_and_update().own_private_bot, None);

    let current = *crate::chat::lock(&chat.bots.own_epoch);
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
    let stale = *crate::chat::lock(&bots.own_epoch);
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
    let stale = *crate::chat::lock(&bots.own_epoch);
    // 换服务器时 scope 复用同一 sender 并写入新作用域的初始值，然后旧链接才析构。
    let initial = watch.borrow().clone();
    sender.send_replace(initial);
    drop(chat);
    bots.publish_own_private_bot(stale, Ok(own_bot_ready()));
    assert_eq!(watch.borrow_and_update().own_private_bot, None);
}

fn own_bot_problem() -> wonton_protocol::OwnBotProblem {
    wonton_protocol::OwnBotProblem {
        category: wonton_protocol::OwnBotFailure::TransportUnavailable,
        kind: None,
        status: None,
        detail: "传输不可用".into(),
    }
}

#[tokio::test]
async fn foregrounded_reissues_the_own_bot_read_and_clears_a_stale_problem() {
    let chat = rig().0;
    let mut watch = chat.bots.view.subscribe();
    let initial_epoch = *crate::chat::lock(&chat.bots.own_epoch);
    chat.bots
        .publish_own_private_bot(initial_epoch, Err(own_bot_problem()));
    assert!(watch.borrow_and_update().own_private_bot_problem.is_some());

    chat.bots.foregrounded();
    let retry_epoch = *crate::chat::lock(&chat.bots.own_epoch);
    assert!(retry_epoch > initial_epoch);
    chat.bots
        .publish_own_private_bot(retry_epoch, Ok(own_bot_ready()));
    let view = watch.borrow_and_update();
    assert!(view.own_private_bot_problem.is_none());
    assert!(matches!(
        view.own_private_bot,
        Some(wonton_protocol::OwnPrivateBotView::Ready { .. })
    ));

    let current = *crate::chat::lock(&chat.bots.own_epoch);
    assert_eq!(current, retry_epoch);
}

#[tokio::test]
async fn a_result_left_over_from_the_previous_foreground_cannot_write_back() {
    let chat = rig().0;
    let mut watch = chat.bots.view.subscribe();
    let before = *crate::chat::lock(&chat.bots.own_epoch);
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
    let epoch = *crate::chat::lock(&chat.bots.own_epoch);
    chat.bots
        .publish_own_private_bot(epoch, Ok(own_bot_ready()));
    assert!(watch.borrow_and_update().own_private_bot.is_some());

    chat.bots.foregrounded();
    let view = watch.borrow_and_update();
    assert!(view.own_private_bot.is_none());
    assert!(view.own_private_bot_problem.is_none());
}
