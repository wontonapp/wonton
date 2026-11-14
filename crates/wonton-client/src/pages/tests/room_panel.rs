//! 房间面板的编辑与展示：单元级的行为钉子。

use super::*;

#[tokio::test]
async fn room_panel_view_reports_selection_and_blocking() {
    let rig = Rig::new();
    let room: wonton_matrix::OwnedRoomId = "!r:home.example".parse().unwrap();
    rig.tabs
        .receive_room_panel(
            room.clone(),
            RoomPanel {
                pages: vec![wonton_protocol::PanelEntry {
                    tab: url_tab("https://example.com/room"),
                    title: None,
                }],
            },
        )
        .unwrap();
    rig.tabs.recompute();
    let view = rig.tabs.room_view(room.as_str()).unwrap();
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
    assert!(rig.tabs.room_view(room.as_str()).unwrap().blocked);

    rig.tabs
        .dispatch(Action::SelectPanelPage {
            scope: PanelScope::Room {
                room_id: room.to_string(),
            },
            tab: url_tab("https://example.com/room"),
        })
        .unwrap();
    assert_eq!(
        rig.tabs.room_view(room.as_str()).unwrap().selected,
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
    rig.tabs
        .receive_room_panel(room, RoomPanel::default())
        .unwrap();
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
    rig.tabs
        .receive_room_panel(
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
        )
        .unwrap();
    rig.tabs.recompute();
    rig.plugins.send_replace(PluginsViewModel {
        installed: Some(wonton_protocol::InstalledPlugins {
            plugins: Vec::new(),
            upgrade_check: None,
        }),
        ..plugins("gone")
    });
    rig.tabs.recompute();
    let view = rig.tabs.room_view(room.as_str()).unwrap();
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
async fn two_room_panels_keep_their_pages_selection_and_blocking() {
    let rig = Rig::new();
    let first: OwnedRoomId = "!first:home.example".parse().unwrap();
    let second: OwnedRoomId = "!second:home.example".parse().unwrap();
    let first_tab = url_tab("https://example.com/first");
    let second_tab = url_tab("https://example.com/second");
    for (room, tab) in [(&first, &first_tab), (&second, &second_tab)] {
        rig.tabs
            .receive_room_panel(
                room.clone(),
                RoomPanel {
                    pages: vec![PanelEntry {
                        tab: tab.clone(),
                        title: None,
                    }],
                },
            )
            .unwrap();
    }
    rig.tabs
        .dispatch(Action::SelectPanelPage {
            scope: PanelScope::Room {
                room_id: first.to_string(),
            },
            tab: first_tab.clone(),
        })
        .unwrap();
    rig.tabs
        .dispatch(Action::SetRoomEntryBlocked {
            room_id: second.to_string(),
            blocked: true,
        })
        .unwrap();
    let a = rig.tabs.room_view(first.as_str()).unwrap();
    let b = rig.tabs.room_view(second.as_str()).unwrap();
    assert_eq!(a.panel[0].tab, first_tab);
    assert_eq!(a.selected, Some(first_tab));
    assert!(!a.blocked);
    assert_eq!(b.panel[0].tab, second_tab);
    assert_eq!(b.selected, None);
    assert!(b.blocked);
    assert!(rig.tabs.room_view("!missing:home.example").is_none());
}
