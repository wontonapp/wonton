//! 房间面板的编辑与展示：单元级的行为钉子。

use super::*;

#[tokio::test]
async fn room_panel_view_reports_selection_and_blocking() {
    let rig = Rig::new();
    let room: wonton_matrix::OwnedRoomId = "!r:home.example".parse().unwrap();
    rig.room_panel.send_replace(Some((
        room.clone(),
        RoomPanel {
            pages: vec![wonton_protocol::PanelEntry {
                tab: url_tab("https://example.com/room"),
                title: None,
            }],
        },
    )));
    rig.tabs.recompute();
    let view = rig.tabs.room_view.borrow().clone().unwrap();
    assert_eq!(view.panel.len(), 1);
    assert_eq!(view.panel[0].page.title, "example.com");
    assert_eq!(view.selected, None);
    assert!(!view.blocked);

    rig.tabs
        .dispatch(Action::SetRoomEntryBlocked {
            room_id: room.to_string(),
            blocked: true,
        })
        .unwrap();
    assert!(rig.tabs.room_view.borrow().clone().unwrap().blocked);

    rig.tabs
        .dispatch(Action::SelectPanelPage {
            scope: PanelScope::Room {
                room_id: room.to_string(),
            },
            tab: url_tab("https://example.com/room"),
        })
        .unwrap();
    assert_eq!(
        rig.tabs.room_view.borrow().clone().unwrap().selected,
        Some(url_tab("https://example.com/room"))
    );
}

#[tokio::test]
async fn room_panel_edits_need_an_open_room_and_a_login() {
    let rig = Rig::new();
    let scope = PanelScope::Room {
        room_id: "!r:home.example".to_owned(),
    };
    let closed = rig
        .tabs
        .dispatch(Action::AddPanelPage {
            scope: scope.clone(),
            tab: url_tab("https://example.com/x"),
        })
        .unwrap_err();
    assert!(matches!(closed, CoreError::InvalidInput { .. }));

    let room: wonton_matrix::OwnedRoomId = "!r:home.example".parse().unwrap();
    rig.room_panel
        .send_replace(Some((room, RoomPanel::default())));
    rig.tabs.recompute();
    let signed_out = rig
        .tabs
        .dispatch(Action::AddPanelPage {
            scope,
            tab: url_tab("https://example.com/x"),
        })
        .unwrap_err();
    assert!(matches!(signed_out, CoreError::NotReady { .. }));
}

#[tokio::test]
async fn room_panel_pages_disappear_with_their_plugin() {
    let rig = Rig::new();
    let room: wonton_matrix::OwnedRoomId = "!r:home.example".parse().unwrap();
    rig.room_panel.send_replace(Some((
        room.clone(),
        RoomPanel {
            pages: vec![
                wonton_protocol::PanelEntry {
                    tab: page_tab(),
                    title: None,
                },
                wonton_protocol::PanelEntry {
                    tab: url_tab("https://example.com/keep"),
                    title: None,
                },
            ],
        },
    )));
    rig.tabs.recompute();
    rig.plugins.send_replace(PluginsViewModel {
        installed: Some(wonton_protocol::InstalledPlugins {
            plugins: Vec::new(),
            upgrade_check: None,
        }),
        ..plugins("gone")
    });
    rig.tabs.recompute();
    let view = rig.tabs.room_view.borrow().clone().unwrap();
    assert_eq!(
        view.panel
            .iter()
            .map(|item| item.tab.clone())
            .collect::<Vec<_>>(),
        vec![url_tab("https://example.com/keep")]
    );
    assert!(
        view.panel
            .first()
            .is_some_and(|item| item.page.title == "example.com")
    );
}

#[tokio::test]
async fn room_page_bridge_rejects_departed_rooms_with_the_same_page() {
    let rig = Rig::new();
    let first: OwnedRoomId = "!first:home.example".parse().unwrap();
    let second: OwnedRoomId = "!second:home.example".parse().unwrap();
    let placed = |room: &OwnedRoomId| PlacedTab::Panel {
        scope: PanelScope::Room {
            room_id: room.to_string(),
        },
        tab: page_tab(),
    };
    let panel = RoomPanel {
        pages: vec![PanelEntry {
            tab: page_tab(),
            title: None,
        }],
    };
    rig.room_panel
        .send_replace(Some((first.clone(), panel.clone())));
    rig.tabs.recompute();
    assert!(bridge::page_of(&rig.tabs, &placed(&first)).is_ok());

    rig.room_panel.send_replace(Some((second.clone(), panel)));
    rig.tabs.recompute();
    assert!(matches!(
        bridge::page_of(&rig.tabs, &placed(&first)),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(bridge::page_of(&rig.tabs, &placed(&second)).is_ok());

    rig.room_panel.send_replace(None);
    rig.tabs.recompute();
    assert!(matches!(
        bridge::page_of(&rig.tabs, &placed(&second)),
        Err(CoreError::InvalidInput { .. })
    ));
}
