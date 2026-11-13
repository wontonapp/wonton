use axum::http::Method;
use serde_json::{Value, json};
use wonton_client::{TabBridgeAction, TabBridgeResult, TabState};

use super::support::{PAGE_ORIGIN, bar, environment, open_page, selected_page, wait_quiet};

fn page_requests() -> [Value; 4] {
    [
        json!({"jsonrpc":"2.0","id":1,"method":"ui/open-link","params":{"url":"https://example.org/"}}),
        json!({"jsonrpc":"2.0","id":2,"method":"ui/message","params":{"role":"user","content":[{"type":"text","text":"draft"}]}}),
        json!({"jsonrpc":"2.0","id":3,"method":"ui/request-display-mode","params":{"mode":"fullscreen"}}),
        json!({"jsonrpc":"2.0","method":"ui/notifications/initialized"}),
    ]
}

fn assert_ignored(reply: TabBridgeResult) {
    assert!(reply.reply_script.is_none());
    assert!(reply.action.is_none());
    assert!(!reply.initialized);
}

#[tokio::test]
async fn missing_page_url_ignores_messages_until_plugin_list_arrives() {
    let (mut rig, tab) = open_page(true).await;
    wait_quiet(&rig).await;
    rig.core.hold(Method::GET, "/_wonton/api/plugins");
    std::fs::remove_file(rig.data_dir().join("installed-plugins.json")).unwrap();
    rig.reopen();
    // 重启后栈是空的：重新打开这一页，列表没读到前按存的元数据如实说加载中。
    rig.app
        .dispatch(wonton_client::Action::OpenPage { tab: tab.clone() })
        .unwrap();
    let loading = selected_page(&rig, &tab);
    assert_eq!(loading.state, TabState::Loading);
    assert!(loading.url.is_none());
    let before = rig.app.pages();
    for message in page_requests() {
        assert_ignored(rig.app.tab_handle_message(
            bar(&tab),
            true,
            PAGE_ORIGIN.to_owned(),
            message.to_string(),
            environment(),
        ));
    }
    assert_eq!(rig.app.pages(), before);
    rig.core.release(Method::GET, "/_wonton/api/plugins");
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while selected_page(&rig, &tab).url.is_none() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let reply = rig.app.tab_handle_message(
        bar(&tab),
        true,
        PAGE_ORIGIN.to_owned(),
        page_requests()[0].to_string(),
        environment(),
    );
    assert_eq!(
        reply.action,
        Some(TabBridgeAction::OpenLink {
            url: "https://example.org/".to_owned()
        })
    );
}

#[tokio::test]

async fn matching_origin_accepts_implicit_and_explicit_default_ports() {
    let (rig, tab) = open_page(true).await;
    for origin in [PAGE_ORIGIN, "http://APP.FEED.wonton.localhost:80"] {
        let reply = rig.app.tab_handle_message(
            bar(&tab),
            true,
            origin.to_owned(),
            page_requests()[0].to_string(),
            environment(),
        );
        assert_eq!(
            reply.action,
            Some(TabBridgeAction::OpenLink {
                url: "https://example.org/".to_owned()
            })
        );
    }
}
