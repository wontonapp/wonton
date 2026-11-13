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

#[test]
fn a_late_write_does_not_reopen_a_closed_room() {
    let room: OwnedRoomId = "!old:example.com".parse().unwrap();
    let sender = watch::Sender::new(Some((room.clone(), RoomPanel::default())));
    sender.send_replace(None);
    publish_written_panel(&sender, &room, panel());
    assert!(sender.borrow().is_none());
}

#[test]
fn a_late_write_does_not_replace_the_new_room() {
    let old: OwnedRoomId = "!old:example.com".parse().unwrap();
    let new: OwnedRoomId = "!new:example.com".parse().unwrap();
    let sender = watch::Sender::new(Some((new.clone(), RoomPanel::default())));
    publish_written_panel(&sender, &old, panel());
    let current = sender.borrow();
    let (room, panel) = current.as_ref().unwrap();
    assert_eq!(room, &new);
    assert!(panel.pages.is_empty());
}

#[test]
fn a_write_updates_the_still_open_room() {
    let room: OwnedRoomId = "!current:example.com".parse().unwrap();
    let sender = watch::Sender::new(Some((room.clone(), RoomPanel::default())));
    publish_written_panel(&sender, &room, panel());
    assert_eq!(sender.borrow().as_ref().unwrap().1.pages.len(), 1);
}
