//! 假应用的标签页和顶栏 widget 入口：`TabRef`、`TabPage` 的夹具，以及假 Bot 的网页地址。
//!
//! `fake_open` 只认开得到的标签：应用网格里的网页应用、整页本机网页的那个应用、Bot 私聊；根页面交给
//! 调用方弹回根。

use super::chat::CALL_ROOM_ID;
use wonton_protocol::{
    BotRef, BotWebPage, RoomWidget, TabIcon, TabLayout, TabPage, TabRef, TabState,
};

/// 声明了 `matrix_widget` 的假插件；没有隧道，它的 widget 页停在加载中。
pub(super) const CALL_PLUGIN: &str = "app.example.call";

/// 有网页的假应用：标题、ID，和它的页面停在哪个状态。
pub(super) const WEB_APPS: [(&str, &str, FakePage); 4] = [
    ("Budget", "app.example.budget", FakePage::Loading),
    ("Photos", "app.example.photos", FakePage::Stopped),
    ("Papers", "app.example.papers", FakePage::Failed),
    ("Corner", CORNER_APP, FakePage::Local),
];
/// 点开是一张加载得出来的本机网页（`ui_fake_room::corner`），整页上有真网页可测。
pub(super) const CORNER_APP: &str = "app.example.corner";

#[derive(Clone, Copy)]
pub(super) enum FakePage {
    /// 插件在跑、页面就绪；没有隧道，壳一直画骨架。
    Loading,
    Stopped,
    Failed,
    /// 没有隧道也加载得出来：打开时换成回环地址上的公网网页那一类（`TabRef::Url`），壳直接连。
    Local,
}

pub(super) fn web_app_tab(plugin_id: &str) -> TabRef {
    TabRef::Plugin {
        plugin_id: plugin_id.into(),
        page: None,
    }
}

/// [`CALL_ROOM_ID`] 顶栏的 widget 入口。
pub(in crate::client) fn call_widget() -> RoomWidget {
    RoomWidget {
        tab: TabRef::Widget {
            plugin_id: CALL_PLUGIN.into(),
            room_id: CALL_ROOM_ID.into(),
        },
        title: "Call".into(),
        icon: None,
        entry: wonton_protocol::WidgetEntry::VideoCall,
    }
}

/// 打开 `tab` 时压进栈的那一项；`None` 是根页面。
pub(super) fn fake_open(tab: TabRef) -> Option<(TabRef, TabPage)> {
    if tab == web_app_tab(CORNER_APP) {
        return super::super::ui_fake_room::corner();
    }
    let page = fake_page(&tab)?;
    Some((tab, page))
}

pub(super) fn fake_page(tab: &TabRef) -> Option<TabPage> {
    if let Some(page) = super::music::page(tab) {
        return Some(page);
    }
    if let Some(page) = super::video::page(tab) {
        return Some(page);
    }
    if *tab == call_widget().tab {
        return Some(TabPage {
            title: "Call".into(),
            icon: TabIcon::Plugin { icon: None },
            url: Some(web_url(CALL_PLUGIN)),
            layout: TabLayout::EdgeToEdge,
            state: TabState::Ready,
            reload: 0,
            native: None,
            opens_in_browser: false,
        });
    }
    let TabRef::Plugin { plugin_id, .. } = tab else {
        if let TabRef::Bot { bot, .. } = tab {
            return Some(TabPage {
                title: bot.bot.clone(),
                icon: TabIcon::Plugin { icon: None },
                url: Some(web_url(&bot.plugin_id)),
                layout: TabLayout::default(),
                state: TabState::Ready,
                reload: 0,
                native: None,
                opens_in_browser: false,
            });
        }
        return None;
    };
    let (title, _, kind) = WEB_APPS.iter().find(|(_, id, _)| id == plugin_id)?;
    Some(TabPage {
        title: (*title).into(),
        icon: TabIcon::Plugin { icon: None },
        url: Some(web_url(plugin_id)),
        layout: TabLayout::default(),
        state: match kind {
            FakePage::Loading | FakePage::Local => TabState::Ready,
            FakePage::Stopped => TabState::Stopped,
            FakePage::Failed => TabState::Failed {
                reason: "fake failure from the UI test fixture".into(),
            },
        },
        reload: 0,
        native: None,
        opens_in_browser: false,
    })
}

pub(super) fn web_url(plugin_id: &str) -> String {
    format!("http://{}.wonton.localhost/", plugin_id.replace('.', "-"))
}

pub(super) fn fake_bot_page(plugin_id: &str, bot: &str) -> BotWebPage {
    BotWebPage {
        bot: BotRef {
            plugin_id: plugin_id.into(),
            bot: bot.into(),
        },
        url: web_url(plugin_id),
        cross_origin_isolated: false,
    }
}
