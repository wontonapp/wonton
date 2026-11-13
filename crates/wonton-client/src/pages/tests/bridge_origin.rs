use serde_json::{Value, json};

use super::{Action, PlacedTab, Rig, main_scope, page_tab};
use crate::appearance::AppearanceLink;
use crate::pages::bridge::{
    TabBridgeResult, TabEnvironment, TabPlatform, TabSafeArea, handle_message,
};

const PAGE_ORIGIN: &str = "http://app.shared.wonton.localhost";

fn environment() -> TabEnvironment {
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

fn requests() -> [Value; 5] {
    [
        json!({"jsonrpc":"2.0","id":1,"method":"ui/open-link","params":{"url":"https://example.org/"}}),
        json!({"jsonrpc":"2.0","id":2,"method":"ui/message","params":{"role":"user","content":[{"type":"text","text":"draft"}]}}),
        json!({"jsonrpc":"2.0","id":3,"method":"ui/request-display-mode","params":{"mode":"fullscreen"}}),
        json!({"jsonrpc":"2.0","method":"ui/notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":4,"method":"ui/wonton-attention","params":{"attention":true}}),
    ]
}

fn assert_ignored(reply: TabBridgeResult) {
    assert!(reply.reply_script.is_none());
    assert!(reply.action.is_none());
    assert!(!reply.initialized);
}

#[tokio::test]
async fn foreign_auth_scheme_and_port_origins_have_no_effect() {
    // 页面夹具不启动后台同步；全客户端的 HTTP 总数包含登录和列表刷新，不能归因于桥消息。
    let rig = Rig::new();
    let appearance = AppearanceLink::new(rig._dir.path());
    rig.open(page_tab());
    rig.tabs
        .dispatch(Action::AddPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    let panel = PlacedTab::Panel {
        scope: main_scope(),
        tab: page_tab(),
    };
    let before = rig.view();
    assert!(!before.panel[0].attention);
    for placed in [PlacedTab::Page { tab: page_tab() }, panel.clone()] {
        for origin in [
            "http://other.wonton.localhost",
            "http://auth.wonton.localhost",
            "https://app.shared.wonton.localhost",
            "http://app.shared.wonton.localhost:8080",
            "http://app.shared.wonton.localhost.evil.example",
            "null",
            "",
        ] {
            for mut message in requests() {
                message["origin"] = PAGE_ORIGIN.into();
                assert_ignored(handle_message(
                    &rig.tabs,
                    &appearance,
                    &placed,
                    true,
                    origin,
                    &message.to_string(),
                    &environment(),
                ));
            }
            assert_ignored(handle_message(
                &rig.tabs,
                &appearance,
                &placed,
                true,
                origin,
                "{",
                &environment(),
            ));
        }
    }
    assert_eq!(rig.view(), before);

    let accepted = handle_message(
        &rig.tabs,
        &appearance,
        &panel,
        true,
        PAGE_ORIGIN,
        &requests()[4].to_string(),
        &environment(),
    );
    assert!(accepted.reply_script.is_some());
    assert!(rig.view().panel[0].attention);
}
