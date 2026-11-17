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
}
