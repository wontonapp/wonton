use super::*;
use wonton_protocol::{BotRef, BotView, BotWebPage, BotWebView};

fn bot(name: &str, url: &str) -> BotView {
    let bot = BotRef {
        plugin_id: "app.shared".into(),
        bot: name.into(),
    };
    BotView {
        user_id: format!("@{name}:home"),
        bot: bot.clone(),
        name: name.into(),
        web: Some(BotWebView::Ready {
            page: BotWebPage {
                bot,
                url: url.into(),
                cross_origin_isolated: false,
            },
        }),
        avatar: None,
        state_avatars: Vec::new(),
        activity: None,
        saving: false,
        problem: None,
        chat: Default::default(),
    }
}

fn tab(view: &BotView, path: Option<&str>) -> TabRef {
    TabRef::Bot {
        bot: view.bot.clone(),
        path: path.map(str::to_owned),
    }
}

fn publish(rig: &Rig, bots: Vec<BotView>) {
    rig.chat.send_replace(ChatViewModel {
        bots,
        ..empty_chat()
    });
    rig.tabs.recompute();
}

#[tokio::test]
async fn exact_bot_ready_metadata_drives_stack_panel_and_bridge() {
    let rig = Rig::new();
    let family = bot(
        "family",
        "https://bot--family-app-shared-12345678.example/start",
    );
    let other = bot("other", "https://bot--other-app-shared-87654321.example/");
    publish(&rig, vec![other, family.clone()]);
    let identity = tab(&family, None);
    rig.tabs
        .dispatch(Action::OpenPage {
            tab: identity.clone(),
        })
        .unwrap();
    let view = rig.tabs.view();
    let open = view.stack.last().unwrap();
    assert_eq!(open.tab, identity);
    assert_eq!(
        open.page.url.as_deref(),
        Some("https://bot--family-app-shared-12345678.example/start")
    );
    rig.tabs
        .dispatch(Action::AddPanelPage {
            scope: PanelScope::Main,
            tab: identity.clone(),
        })
        .unwrap();
    let view = rig.tabs.view();
    assert_eq!(view.panel[0].tab, identity);
    assert_eq!(view.panel[0].page.url, view.stack.last().unwrap().page.url);
    let placed = PlacedTab::Panel {
        scope: PanelScope::Main,
        tab: identity.clone(),
    };
    assert_eq!(
        bridge::page_of(&rig.tabs, &placed).unwrap().url,
        view.panel[0].page.url
    );
    let reload = view.panel[0].page.reload;
    publish(&rig, Vec::new());
    let revoked = rig.tabs.view();
    assert_eq!(revoked.panel[0].tab, identity);
    assert_eq!(revoked.panel[0].page.url, None);
    assert_eq!(bridge::page_of(&rig.tabs, &placed).unwrap().url, None);
    assert!(revoked.panel[0].page.reload > reload);
    assert_eq!(revoked.stack.last().unwrap().page.url, None);
    // 已核实的页面身份之外的路径不收：绝对网址不能当 Bot 页的路径。
    assert!(
        rig.tabs
            .dispatch(Action::OpenPage {
                tab: tab(&family, Some("/new"))
            })
            .is_err()
    );
}

#[tokio::test]
async fn pin_current_page_keeps_default_and_nondefault_bot_identity_and_paths() {
    let rig = Rig::new();
    let default = bot("default", "http://app.shared.wonton.localhost/");
    let family = bot("family", "https://bot--family-app-shared-12345678.example/");
    publish(&rig, vec![default.clone(), family.clone()]);
    for (bot, url) in [
        (
            &default,
            "http://app.shared.wonton.localhost/report?q=1#now",
        ),
        (
            &family,
            "https://bot--family-app-shared-12345678.example/report?q=1#now",
        ),
    ] {
        rig.tabs
            .dispatch(Action::PinCurrentPage {
                url: url.into(),
                title: None,
                to: PinTarget::Panel {
                    scope: PanelScope::Main,
                },
            })
            .unwrap();
        assert_eq!(
            rig.tabs.view().panel.last().unwrap().tab,
            tab(bot, Some("/report?q=1#now"))
        );
    }
    rig.tabs
        .dispatch(Action::OpenPage {
            tab: tab(&default, None),
        })
        .unwrap();
    let mut revoked = default.clone();
    revoked.web = None;
    publish(&rig, vec![revoked, family]);
    // 资料撤回后，同一网址不再钉成 Bot 页（也没有普通插件页可退）。
    assert!(
        rig.tabs
            .dispatch(Action::PinCurrentPage {
                url: "http://app.shared.wonton.localhost/new".into(),
                title: None,
                to: PinTarget::Panel {
                    scope: PanelScope::Main,
                },
            })
            .is_err()
    );
    assert_eq!(
        rig.tabs.view().panel[0].tab,
        tab(&default, Some("/report?q=1#now"))
    );
    assert_eq!(rig.tabs.view().panel[0].page.url, None);
}

#[tokio::test]
async fn mismatched_page_identity_unknown_and_disconnected_bot_never_borrow_plugin_url() {
    let rig = Rig::new();
    let family = bot("family", "https://bot--family-app-shared-12345678.example/");
    publish(&rig, vec![family.clone()]);
    rig.tabs
        .dispatch(Action::OpenPage {
            tab: tab(&family, None),
        })
        .unwrap();
    let mut mismatched = family.clone();
    if let Some(BotWebView::Ready { page }) = &mut mismatched.web {
        page.bot.bot = "default".into();
    }
    publish(&rig, vec![mismatched]);
    assert_eq!(rig.tabs.view().stack.last().unwrap().page.url, None);
    let mut unknown = family.clone();
    unknown.web = Some(BotWebView::Other);
    publish(&rig, vec![unknown]);
    assert_eq!(rig.tabs.view().stack.last().unwrap().page.url, None);
    publish(&rig, vec![family]);
    let mut disconnected = home("old-server");
    disconnected.status = HomeStatus::Disconnected {
        reason: wonton_protocol::HomeFailure::TransportUnavailable,
        message: String::new(),
    };
    rig.home.send_replace(disconnected);
    rig.tabs.recompute();
    assert_eq!(rig.tabs.view().stack.last().unwrap().page.url, None);
}

#[tokio::test]
async fn bot_room_panels_keep_identity_but_reject_outside_or_absolute_paths() {
    let rig = Rig::new();
    let family = bot(
        "family",
        "https://bot--family-app-shared-12345678.example/start",
    );
    publish(&rig, vec![family.clone()]);
    for path in [
        "https://evil.example/x",
        "https://bot--family-app-shared-12345678.example/x",
    ] {
        assert!(
            rig.tabs
                .dispatch(Action::OpenPage {
                    tab: tab(&family, Some(path)),
                })
                .is_err()
        );
    }
    let room: OwnedRoomId = "!bot:home".parse().unwrap();
    rig.room_panel.send_replace(Some((
        room,
        RoomPanel {
            pages: vec![PanelEntry {
                tab: tab(&family, Some("/report?q=1#now")),
                title: None,
            }],
        },
    )));
    rig.tabs.recompute();
    let view = rig.tabs.room_view().unwrap();
    assert_eq!(view.panel[0].tab, tab(&family, Some("/report?q=1#now")));
    assert_eq!(
        view.panel[0].page.url.as_deref(),
        Some("https://bot--family-app-shared-12345678.example/report?q=1#now")
    );
}

#[tokio::test]
async fn explicit_no_bot_webui_preserves_ordinary_plugin_current_page_pinning() {
    let rig = Rig::new();
    let mut default = bot("default", "http://app.shared.wonton.localhost/");
    default.web = Some(BotWebView::Unavailable {
        reason: wonton_protocol::BotWebFailure::NoWebui,
    });
    publish(&rig, vec![default]);
    rig.tabs
        .dispatch(Action::PinCurrentPage {
            url: "http://app.shared.wonton.localhost/report".into(),
            title: None,
            to: PinTarget::Panel {
                scope: PanelScope::Main,
            },
        })
        .unwrap();
    assert_eq!(
        rig.tabs.view().panel[0].tab,
        TabRef::PluginPath {
            plugin_id: "app.shared".into(),
            path: "/report".into(),
        }
    );
}
