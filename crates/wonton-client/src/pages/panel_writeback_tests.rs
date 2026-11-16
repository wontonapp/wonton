use super::super::tests::Rig;
use super::*;

fn panel() -> RoomPanel {
    RoomPanel {
        pages: vec![PanelEntry {
            tab: TabRef::Url {
                url: "https://example.com/report".to_owned(),
            },
            title: None,
        }],
    }
}

fn channel(rig: &Rig, room: &OwnedRoomId) -> watch::Sender<Option<Arc<RoomPanelView>>> {
    lock(&rig.tabs.state).rooms[room.as_str()].view.clone()
}

#[tokio::test]
async fn a_late_write_does_not_reopen_a_closed_room() {
    let rig = Rig::new();
    let room: OwnedRoomId = "!old:example.com".parse().unwrap();
    rig.open_room(&room, RoomPanel::default());
    let expected = channel(&rig, &room);
    rig.close_room(&room);
    rig.tabs
        .receive_room_panel(&room, panel(), &expected)
        .unwrap();
    assert!(rig.tabs.room_view(room.as_str()).is_none());
}

#[tokio::test]
async fn a_late_write_does_not_replace_a_reopened_room() {
    let rig = Rig::new();
    let room: OwnedRoomId = "!room:example.com".parse().unwrap();
    rig.open_room(&room, RoomPanel::default());
    let expected = channel(&rig, &room);
    rig.close_room(&room);
    rig.open_room(&room, RoomPanel::default());
    rig.tabs
        .receive_room_panel(&room, panel(), &expected)
        .unwrap();
    assert!(rig.tabs.room_view(room.as_str()).unwrap().panel.is_empty());
}

#[tokio::test]
async fn a_write_updates_the_still_open_room() {
    let rig = Rig::new();
    let room: OwnedRoomId = "!current:example.com".parse().unwrap();
    rig.open_room(&room, RoomPanel::default());
    let expected = channel(&rig, &room);
    rig.tabs
        .receive_room_panel(&room, panel(), &expected)
        .unwrap();
    assert_eq!(rig.tabs.room_view(room.as_str()).unwrap().panel.len(), 1);
}
