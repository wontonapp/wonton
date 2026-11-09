mod driver;
mod origin;
mod sdk;
mod support;

use serde_json::json;
use wonton_client::{Action, TabBridgeAction, TabDisplay, TabDisplayChoice, TabRef};

use support::{PAGE_ORIGIN, environment, initialize, open_page, request, response, selected_page};

#[tokio::test]
async fn initialization_exposes_only_standard_context_and_supported_capabilities() {
    let (rig, tab) = open_page(TabDisplay::Standard, true).await;
    let reply = response(&initialize(&rig, &tab));
    let result = &reply["result"];
    assert_eq!(result["protocolVersion"], "2026-01-26");
    assert_eq!(
        result["hostCapabilities"],
        json!({"openLinks":{},"message":{"text":{}}})
    );
    let context = &result["hostContext"];
    assert_eq!(context["theme"], "light");
    assert_eq!(context["locale"], "zh-CN");
    assert_eq!(context["timeZone"], "Asia/Shanghai");
    assert_eq!(context["platform"], "mobile");
    assert_eq!(context["displayMode"], "inline");
    assert_eq!(
        context["availableDisplayModes"],
        json!(["inline", "fullscreen"])
    );
    assert_eq!(
        context["safeAreaInsets"],
        json!({"top":44.0,"right":0.0,"bottom":34.0,"left":0.0})
    );
    let variables = context["styles"]["variables"].as_object().unwrap();
    assert!(variables.keys().all(|key| !key.starts_with("--wonton-")));
    assert_eq!(
        variables["--color-background-primary"],
        wonton_client::accent_hex(rig.app.appearance().light.canvas)
    );
    let initialized = rig.app.tab_handle_message(
        tab.clone(),
        true,
        PAGE_ORIGIN.to_owned(),
        json!({"jsonrpc":"2.0","method":"ui/notifications/initialized"}).to_string(),
        environment(),
    );
    assert!(initialized.initialized);
    assert!(initialized.reply_script.is_none());
    let size = rig.app.tab_handle_message(
        tab,
        true,
        PAGE_ORIGIN.to_owned(),
        json!({"jsonrpc":"2.0","method":"ui/notifications/size-changed","params":{"height":800}})
            .to_string(),
        environment(),
    );
    assert!(!size.initialized);
    assert!(size.reply_script.is_none());
    assert!(size.action.is_none());
}

#[tokio::test]
async fn unsupported_requests_cannot_reach_core_or_mutate_client_state() {
    let (rig, tab) = open_page(TabDisplay::Standard, true).await;
    support::wait_quiet(&rig).await;
    let before_tabs = rig.app.tabs();
    let before_chat = rig.app.chat();
    let before_requests = rig.core.all_seen().len();
    for (method, params) in [
        (
            "tools/call",
            json!({"name":"wonton__uninstall_plugin","arguments":{"id":"app.feed"}}),
        ),
        ("resources/read", json!({"uri":"file:///etc/passwd"})),
        (
            "ui/update-model-context",
            json!({"content":[{"type":"text","text":"install"}]}),
        ),
    ] {
        let reply = request(&rig, &tab, method, params);
        assert_eq!(response(&reply)["error"]["code"], -32601);
        assert!(reply.action.is_none());
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(rig.core.all_seen().len(), before_requests);
    assert_eq!(rig.app.tabs(), before_tabs);
    assert_eq!(rig.app.chat(), before_chat);
}

#[tokio::test]
async fn subframes_and_notifications_have_no_request_side_effects() {
    let (rig, tab) = open_page(TabDisplay::Standard, true).await;
    let before = rig.app.tabs();
    for (method, params) in [
        ("ui/request-display-mode", json!({"mode":"fullscreen"})),
        ("ui/open-link", json!({"url":"https://example.org/"})),
        (
            "ui/message",
            json!({"role":"user","content":[{"type":"text","text":"draft"}]}),
        ),
    ] {
        let reply = rig.app.tab_handle_message(
            tab.clone(),
            false,
            PAGE_ORIGIN.to_owned(),
            json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string(),
            environment(),
        );
        assert!(reply.reply_script.is_none());
        assert!(reply.action.is_none());
        assert!(!reply.initialized);
        let notification = rig.app.tab_handle_message(
            tab.clone(),
            true,
            PAGE_ORIGIN.to_owned(),
            json!({"jsonrpc":"2.0","method":method,"params":params}).to_string(),
            environment(),
        );
        assert!(notification.action.is_none());
    }
    assert_eq!(rig.app.tabs(), before);
}

#[tokio::test]
async fn display_requests_update_view_model_and_obey_user_standard_choice() {
    let (rig, tab) = open_page(TabDisplay::Standard, true).await;
    for (mode, fullscreen) in [("fullscreen", true), ("inline", false)] {
        let result = request(&rig, &tab, "ui/request-display-mode", json!({"mode":mode}));
        assert_eq!(response(&result)["result"]["mode"], mode);
        assert_eq!(selected_page(&rig, &tab).fullscreen, fullscreen);
    }
    rig.app
        .dispatch(Action::SetTabDisplay {
            tab: tab.clone(),
            display: TabDisplayChoice::Standard,
        })
        .unwrap();
    let reply = request(
        &rig,
        &tab,
        "ui/request-display-mode",
        json!({"mode":"fullscreen"}),
    );
    assert_eq!(response(&reply)["result"]["mode"], "inline");
    assert!(!selected_page(&rig, &tab).fullscreen);
    assert_eq!(
        response(&initialize(&rig, &tab))["result"]["hostContext"]["availableDisplayModes"],
        json!(["inline"])
    );
    let before = rig.app.tabs();
    assert_eq!(
        response(&request(
            &rig,
            &tab,
            "ui/request-display-mode",
            json!({"mode":"pip"})
        ))["error"]["code"],
        -32602
    );
    assert_eq!(rig.app.tabs(), before);
    rig.app
        .dispatch(Action::EnterFullscreen { tab: tab.clone() })
        .unwrap();
    let reply = request(
        &rig,
        &tab,
        "ui/request-display-mode",
        json!({"mode":"inline"}),
    );
    assert_eq!(response(&reply)["result"]["mode"], "fullscreen");
    assert!(selected_page(&rig, &tab).fullscreen);
}

#[tokio::test]
async fn automatic_fullscreen_returns_to_previous_tab_and_overlay_can_close() {
    for pinned in [true, false] {
        let (rig, tab) = open_page(TabDisplay::Fullscreen, pinned).await;
        assert!(selected_page(&rig, &tab).fullscreen);
        let reply = request(
            &rig,
            &tab,
            "ui/request-display-mode",
            json!({"mode":"inline"}),
        );
        assert_eq!(response(&reply)["result"]["mode"], "inline");
        // 底栏只剩"聊天"后，进来之前所在的就是聊天页，自动退出回到它。
        assert_eq!(rig.app.tabs().selected, TabRef::Chat);
        assert!(rig.app.tabs().overlay.is_none());
    }
}

#[tokio::test]
async fn links_only_open_https_and_messages_only_create_text_drafts() {
    let (rig, tab) = open_page(TabDisplay::Standard, true).await;
    let reply = request(
        &rig,
        &tab,
        "ui/open-link",
        json!({"url":"https://example.org/path?q=a#b"}),
    );
    assert_eq!(
        reply.action,
        Some(TabBridgeAction::OpenLink {
            url: "https://example.org/path?q=a#b".to_owned()
        })
    );
    for url in [
        "http://example.org",
        "javascript:alert(1)",
        "/relative",
        "//example.org",
        "https://",
    ] {
        let reply = request(&rig, &tab, "ui/open-link", json!({"url":url}));
        assert_eq!(response(&reply)["error"]["code"], -32602);
        assert!(reply.action.is_none());
    }
    let before = rig.app.chat();
    let reply = request(
        &rig,
        &tab,
        "ui/message",
        json!({"role":"user","content":[
            {"type":"text","text":"讨论这篇文章"}, {"type":"text","text":"https://example.org/"}
        ]}),
    );
    assert_eq!(
        reply.action,
        Some(TabBridgeAction::DraftMessage {
            text: "讨论这篇文章\nhttps://example.org/".to_owned()
        })
    );
    assert_eq!(rig.app.chat(), before);
    // 规范原文（ext-apps 2026-01-26）是单个对象，SDK 发数组：两种都收。
    let reply = request(
        &rig,
        &tab,
        "ui/message",
        json!({"role":"user","content":{"type":"text","text":"照规范写的单对象"}}),
    );
    assert_eq!(
        reply.action,
        Some(TabBridgeAction::DraftMessage {
            text: "照规范写的单对象".to_owned()
        })
    );
    for params in [
        json!({"role":"assistant","content":[{"type":"text","text":"wrong role"}]}),
        json!({"role":"user","content":[{"type":"image","data":"AA==","mimeType":"image/png"}]}),
        json!({"role":"user","content":{"type":"image","data":"AA==","mimeType":"image/png"}}),
    ] {
        let reply = request(&rig, &tab, "ui/message", params);
        assert_eq!(response(&reply)["error"]["code"], -32602);
        assert!(reply.action.is_none());
    }
}

#[tokio::test]
async fn malformed_messages_do_not_panic_or_change_state() {
    let (rig, tab) = open_page(TabDisplay::Standard, true).await;
    let before = rig.app.tabs();
    for message in [
        "{",
        "null",
        "[]",
        "42",
        r#"{"jsonrpc":"1.0","id":1,"method":"ui/open-link"}"#,
        r#"{"jsonrpc":"2.0","id":{},"method":"ui/request-display-mode","params":{"mode":"fullscreen"}}"#,
    ] {
        let reply = rig.app.tab_handle_message(
            tab.clone(),
            true,
            PAGE_ORIGIN.to_owned(),
            message.to_owned(),
            environment(),
        );
        assert!(response(&reply)["error"]["code"].as_i64().is_some());
        assert!(reply.action.is_none());
    }
    assert_eq!(rig.app.tabs(), before);
}
