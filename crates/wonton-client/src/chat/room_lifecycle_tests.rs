use super::*;
use crate::chat::{ChatChannels, ChatLink, target::TimelineOpening};
use wonton_protocol::{PushMethod, RoomPanel, TimelineTarget};

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
            room_panel_out: watch::Sender::new(None),
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

fn subscription(opening: TimelineOpening) -> TimelineSubscription {
    TimelineSubscription {
        updates: Mutex::new(mpsc::unbounded_channel().1),
        cancelled: watch::Sender::new(false),
        scroll: watch::Sender::new(TimelineScroll::Away),
        opening: std::sync::Mutex::new(Some(opening)),
    }
}

#[tokio::test]
async fn cancelling_the_room_subscription_closes_its_panel() {
    let (chat, _dir) = rig();
    let opening = begin(&chat, "!room:example.com", None);
    assert!(opening.publish_panel(RoomPanel::default()));
    let subscription = subscription(opening);
    assert!(chat.room_panel_out.borrow().is_some());
    subscription.cancel();
    assert!(chat.room_panel_out.borrow().is_none());
    assert!(subscription.next().await.is_none());
}

#[tokio::test]
async fn a_late_room_open_and_close_cannot_replace_the_new_room() {
    let (chat, _dir) = rig();
    let old = begin(&chat, "!old:example.com", None);
    assert!(old.publish_panel(RoomPanel::default()));
    let new = begin(&chat, "!new:example.com", None);
    assert!(chat.room_panel_out.borrow().is_none());
    assert!(new.publish_panel(RoomPanel::default()));
    assert!(!old.publish_panel(RoomPanel::default()));
    subscription(old).cancel();
    assert_eq!(
        chat.room_panel_out.borrow().as_ref().unwrap().0.as_str(),
        "!new:example.com"
    );
    let new = subscription(new);
    new.cancel();
    assert!(chat.room_panel_out.borrow().is_none());
}

#[tokio::test]
async fn an_old_close_cannot_close_a_reopened_room() {
    let (chat, _dir) = rig();
    let old = begin(&chat, "!room:example.com", None);
    assert!(old.publish_panel(RoomPanel::default()));
    let new = begin(&chat, "!room:example.com", None);
    assert!(new.publish_panel(RoomPanel::default()));
    subscription(old).cancel();
    assert!(chat.room_panel_out.borrow().is_some());
    drop(new);
    assert!(chat.room_panel_out.borrow().is_none());
}

#[tokio::test]
async fn closing_a_side_chat_keeps_the_main_room_open() {
    let (chat, _dir) = rig();
    let main = begin(&chat, "!room:example.com", None);
    assert!(main.publish_panel(RoomPanel::default()));
    let thread = begin(&chat, "!room:example.com", Some("$thread"));
    assert!(thread.publish_panel(RoomPanel::default()));
    subscription(thread).cancel();
    assert!(chat.room_panel_out.borrow().is_some());
    subscription(main).cancel();
    assert!(chat.room_panel_out.borrow().is_none());
}

#[tokio::test]
async fn dropping_the_room_subscription_closes_its_panel() {
    let (chat, _dir) = rig();
    let opening = begin(&chat, "!room:example.com", None);
    assert!(opening.publish_panel(RoomPanel::default()));
    drop(subscription(opening));
    assert!(chat.room_panel_out.borrow().is_none());
}

#[tokio::test]
async fn forgetting_the_session_rejects_a_pending_room_open() {
    let (chat, _dir) = rig();
    let pending = begin(&chat, "!room:example.com", None);
    chat.forget_current().await;
    assert!(!pending.publish_panel(RoomPanel::default()));
    assert!(chat.room_panel_out.borrow().is_none());
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
        )
        .unwrap();
    let update = updates.try_recv().unwrap();
    assert_eq!(update.panel_selected, Some(selected));
    assert!(update.entry_blocked);
}
