use serde_json::{Value, json};
use wonton_client::{
    Action, PlacedTab, PluginImage, PluginTab, TabBridgeResult, TabEnvironment, TabLayout, TabPage,
    TabPlatform, TabRef, TabSafeArea,
};

use crate::plugins::rig::{Rig, running};

pub const PAGE_ORIGIN: &str = "http://app.feed.wonton.localhost";

pub fn environment() -> TabEnvironment {
    TabEnvironment {
        locale: "zh-CN".to_owned(),
        time_zone: "Asia/Shanghai".to_owned(),
        platform: TabPlatform::Ios,
        dark: false,
        inset_top: 88.0,
        inset_bottom: 83.0,
        safe_area: TabSafeArea {
            top: 44.0,
            right: 0.0,
            bottom: 34.0,
            left: 0.0,
        },
    }
}

pub async fn open_page(pinned: bool) -> (Rig, TabRef) {
    let rig = Rig::new().await;
    let mut plugin = running("app.feed");
    plugin.web_url = Some("http://app.feed.wonton.localhost/".to_owned());
    plugin.tabs.push(PluginTab {
        id: "feed".to_owned(),
        title: "Feed".to_owned(),
        icon: PluginImage {
            path: "/_wonton/api/images/feed.png".to_owned(),
        },
        icon_selected: None,
        path: "/feed".to_owned(),
        layout: TabLayout::EdgeToEdge,
    });
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    rig.app.stop_periodic_checks();
    rig.wait_for(|view| view.installed.is_some()).await;
    let tab = TabRef::Plugin {
        plugin_id: "app.feed".to_owned(),
        page: Some("feed".to_owned()),
    };
    if pinned {
        rig.app
            .dispatch(Action::AddMyApp {
                app: wonton_protocol::MyAppRef::Plugin {
                    plugin_id: "app.feed".to_owned(),
                },
                title: None,
                index: None,
            })
            .unwrap();
    }
    rig.app
        .dispatch(Action::OpenPage { tab: tab.clone() })
        .unwrap();
    (rig, tab)
}

/// 底栏上的那一份实例。
pub fn bar(tab: &TabRef) -> PlacedTab {
    PlacedTab::Page { tab: tab.clone() }
}

pub fn request(rig: &Rig, tab: &TabRef, method: &str, params: Value) -> TabBridgeResult {
    rig.app.tab_handle_message(
        bar(tab),
        true,
        PAGE_ORIGIN.to_owned(),
        json!({"jsonrpc":"2.0","id":"request-1","method":method,"params":params}).to_string(),
        environment(),
    )
}

pub fn initialize(rig: &Rig, tab: &TabRef) -> TabBridgeResult {
    request(
        rig,
        tab,
        "ui/initialize",
        json!({
            "protocolVersion":"2026-01-26", "appInfo":{"name":"test", "version":"1"}, "appCapabilities":{}
        }),
    )
}

pub fn response(result: &TabBridgeResult) -> Value {
    let script = result
        .reply_script
        .as_ref()
        .expect("RPC request requires a reply");
    let json = script
        .strip_prefix("window.dispatchEvent(new MessageEvent(\"message\",{data:")
        .unwrap()
        .strip_suffix(",source:window}));")
        .unwrap();
    serde_json::from_str(json).unwrap()
}

/// 栈里那页现在的渲染数据。
pub fn selected_page(rig: &Rig, tab: &TabRef) -> TabPage {
    let view = rig.app.pages();
    view.stack
        .into_iter()
        .find(|open| open.tab == *tab)
        .unwrap()
        .page
}

pub async fn wait_quiet(rig: &Rig) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut previous = rig.core.all_seen().len();
    let mut stable = 0;
    while stable < 5 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "fake core did not settle"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let current = rig.core.all_seen().len();
        if current == previous
            && matches!(
                rig.app.chat().account,
                wonton_client::ChatAccount::Failed { .. }
            )
        {
            stable += 1;
        } else {
            stable = 0;
        }
        previous = current;
    }
}
