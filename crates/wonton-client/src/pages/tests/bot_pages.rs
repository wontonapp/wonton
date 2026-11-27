use super::*;
use wonton_protocol::{BotRef, BotView, BotWebPage, BotWebView};

fn bot(name: &str, url: &str) -> BotView {
    let bot = BotRef {
        plugin_id: "app.shared".into(),
        bot: name.into(),
    };
    let page = BotWebPage {
        bot: bot.clone(),
        url: url.into(),
        cross_origin_isolated: false,
    };
    BotView {
        availability: Default::default(),
        shared: false,
        danger: None,
        user_id: format!("@{name}:home"),
        bot,
        name: name.into(),
        web: Some(BotWebView::Ready { page: page.clone() }),
        web_page: Some(page),
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
        ask: None,
        bots,
        ..empty_chat()
    });
    rig.tabs.recompute();
}

#[tokio::test]
async fn exact_bot_ready_metadata_drives_stack_and_bridge() {
    let rig = Rig::new();
    let family = bot(
        "family",
        "https://bot--family-app-shared-12345678.example/start",
    );
    let other = bot("other", "https://bot--other-app-shared-87654321.example/");
    publish(&rig, vec![other, family.clone()]);
    let identity = tab(&family, None);
    rig.tabs
        .dispatch(PageAction::OpenPage {
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
    let placed = PlacedTab::Page {
        tab: identity.clone(),
    };
    assert_eq!(
        bridge::page_of(&rig.tabs, &placed).unwrap().url,
        open.page.url
    );
    let reload = open.page.reload;
    publish(&rig, Vec::new());
    let revoked = rig.tabs.view();
    let open = revoked.stack.last().unwrap();
    assert_eq!(open.tab, identity);
    assert_eq!(open.page.url, None);
    assert!(open.page.reload > reload);
    assert_eq!(bridge::page_of(&rig.tabs, &placed).unwrap().url, None);
    // 已核实的页面身份之外的路径不收：绝对网址不能当 Bot 页的路径。
    assert!(
        rig.tabs
            .dispatch(PageAction::OpenPage {
                tab: tab(&family, Some("/new"))
            })
            .is_err()
    );
}

#[tokio::test]
async fn mismatched_page_identity_unknown_and_disconnected_bot_never_borrow_plugin_url() {
    let rig = Rig::new();
    let family = bot("family", "https://bot--family-app-shared-12345678.example/");
    publish(&rig, vec![family.clone()]);
    rig.tabs
        .dispatch(PageAction::OpenPage {
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
async fn bot_pages_reject_outside_or_absolute_paths() {
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
                .dispatch(PageAction::OpenPage {
                    tab: tab(&family, Some(path)),
                })
                .is_err()
        );
    }
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
        .dispatch(PageAction::PinCurrentPage {
            url: "http://app.shared.wonton.localhost/report".into(),
            title: None,
        })
        .unwrap();
    assert_eq!(
        rig.last_app_tab(),
        Some(TabRef::PluginPath {
            plugin_id: "app.shared".into(),
            path: "/report".into(),
        })
    );
}
