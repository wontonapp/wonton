//! widget 页（ADR-0096）在没有聊天会话时：页面如实显示打不开，不能固定，订阅不到驱动；
//! 普通插件页不接 widget 报文，照旧当 JSON-RPC 核。真实的驱动和服务器见 `chat::widget`。

use serde_json::json;
use wonton_client::{Action, CoreError, PluginWidget, TabRef, TabState, WidgetEntry};

use super::support::{PAGE_ORIGIN, bar, environment, open_page, selected_page};
use crate::plugins::rig::Rig;

fn widget(rig: &Rig) -> TabRef {
    let mut plugin = crate::plugins::rig::running("app.call");
    plugin.web_url = Some("http://app.call.wonton.localhost/".to_owned());
    plugin.matrix_widget = Some(PluginWidget {
        url: "/room?widgetId=$matrix_widget_id".to_owned(),
        entry: WidgetEntry::VoiceCall,
        capabilities: Vec::new(),
    });
    rig.core.set_lists(vec![vec![plugin]]);
    TabRef::Widget {
        plugin_id: "app.call".to_owned(),
        room_id: "!room:wonton.localhost".to_owned(),
    }
}

#[tokio::test]
async fn a_widget_without_a_chat_session_says_so_and_cannot_be_pinned() {
    let rig = Rig::new().await;
    let tab = widget(&rig);
    rig.connected().await;
    rig.app.stop_periodic_checks();
    rig.wait_for(|view| view.installed.is_some()).await;

    rig.app
        .dispatch(Action::OpenPage { tab: tab.clone() })
        .unwrap();
    let view = rig.app.pages();
    let opened = view.stack.last().expect("widget 页压到栈顶");
    assert_eq!(opened.tab, tab);
    // 驱动在后台起，起不来的原因稍后才写进页面。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let page = selected_page(&rig, &tab);
        if matches!(page.state, TabState::Failed { .. }) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{page:?}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(matches!(
        rig.app.subscribe_tab_widget(bar(&tab)),
        Err(CoreError::NotReady { .. })
    ));

    let pinned = rig.app.dispatch(Action::AddMyApp {
        app: wonton_client::MyAppRef::Page { tab: tab.clone() },
        title: None,
        index: None,
    });
    assert!(
        matches!(pinned, Err(CoreError::InvalidInput { .. })),
        "{pinned:?}"
    );

    let other = TabRef::Url {
        url: "https://example.com/other".to_owned(),
    };
    rig.app
        .dispatch(Action::OpenPage { tab: other.clone() })
        .unwrap();
    assert_eq!(
        rig.app
            .pages()
            .stack
            .iter()
            .map(|open| &open.tab)
            .collect::<Vec<_>>(),
        vec![&tab, &other]
    );
    rig.app
        .dispatch(Action::OpenPage { tab: tab.clone() })
        .unwrap();
    assert_eq!(
        rig.app
            .pages()
            .stack
            .iter()
            .map(|open| &open.tab)
            .collect::<Vec<_>>(),
        vec![&other, &tab]
    );
    let second = TabRef::Widget {
        plugin_id: "app.call".to_owned(),
        room_id: "!second:wonton.localhost".to_owned(),
    };
    rig.app
        .dispatch(Action::OpenPage {
            tab: second.clone(),
        })
        .unwrap();
    assert_eq!(
        rig.app
            .pages()
            .stack
            .iter()
            .map(|open| &open.tab)
            .collect::<Vec<_>>(),
        vec![&other, &second]
    );
    rig.app.dispatch(Action::ClosePage).unwrap();
    assert_eq!(
        rig.app
            .pages()
            .stack
            .iter()
            .map(|open| &open.tab)
            .collect::<Vec<_>>(),
        vec![&other]
    );
}

#[tokio::test]
async fn an_ordinary_page_has_no_widget_channel() {
    let (rig, tab) = open_page(true).await;
    assert!(matches!(
        rig.app.subscribe_tab_widget(bar(&tab)),
        Err(CoreError::InvalidInput { .. })
    ));
    let message = json!({
        "api": "fromWidget", "action": "content_loaded", "requestId": "1", "widgetId": "w", "data": {},
    });
    let result = rig.app.tab_handle_message(
        bar(&tab),
        true,
        PAGE_ORIGIN.to_owned(),
        message.to_string(),
        environment(),
    );
    let reply = super::support::response(&result);
    assert_eq!(reply["error"]["code"], -32600, "{reply}");
}
