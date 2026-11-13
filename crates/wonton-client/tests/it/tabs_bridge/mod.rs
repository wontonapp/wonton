mod driver;
mod origin;
mod sdk;
mod support;

use axum::http::Method;
use serde_json::json;
use wonton_client::{Action, CoreError, PlacedTab, TabRef};
use wonton_protocol::PanelScope;

use crate::plugins::rig::Rig;

use support::{PAGE_ORIGIN, bar, environment, initialize, open_page, request, response};

#[tokio::test]
async fn initialization_exposes_only_standard_context_and_supported_capabilities() {
    let (rig, tab) = open_page(true).await;
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
    // 整页就是全屏（ADR-0095）。
    assert_eq!(context["displayMode"], "fullscreen");
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
        bar(&tab),
        true,
        PAGE_ORIGIN.to_owned(),
        json!({"jsonrpc":"2.0","method":"ui/notifications/initialized"}).to_string(),
        environment(),
    );
    assert!(initialized.initialized);
    assert!(initialized.reply_script.is_none());
    let size = rig.app.tab_handle_message(
        bar(&tab),
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
async fn public_pages_get_no_document_script_or_bridge() {
    let rig = Rig::new().await;
    rig.app.stop_periodic_checks();
    let tab = TabRef::Url {
        url: "https://example.com/news".to_owned(),
    };
    rig.app
        .dispatch(Action::OpenPage { tab: tab.clone() })
        .unwrap();
    rig.app
        .dispatch(Action::AddPanelPage {
            scope: PanelScope::Main,
            tab: tab.clone(),
        })
        .unwrap();

    for placed in [
        bar(&tab),
        PlacedTab::Panel {
            scope: PanelScope::Main,
            tab: tab.clone(),
        },
    ] {
        for error in [
            rig.app.tab_document_script(placed.clone(), environment()),
            rig.app.tab_context_script(placed.clone(), environment()),
        ] {
            assert!(matches!(error, Err(CoreError::InvalidInput { .. })));
        }
        let reply = rig.app.tab_handle_message(
            placed,
            true,
            "https://example.com".to_owned(),
            json!({"jsonrpc":"2.0","id":"draft","method":"ui/message","params":
                {"role":"user","content":[{"type":"text","text":"hello"}]}})
            .to_string(),
            environment(),
        );
        assert!(reply.action.is_none());
        let reply = response(&reply);
        assert_eq!(reply["id"], "draft");
        assert_eq!(reply["error"]["code"], -32001);
    }

    // 通知没有 id，不回话，也不算初始化完成。
    let notification = rig.app.tab_handle_message(
        bar(&tab),
        true,
        "https://example.com".to_owned(),
        json!({"jsonrpc":"2.0","method":"ui/notifications/initialized"}).to_string(),
        environment(),
    );
    assert!(notification.reply_script.is_none());
    assert!(!notification.initialized);
}

#[tokio::test]
async fn unsupported_requests_cannot_reach_core_or_mutate_client_state() {
    let (rig, tab) = open_page(true).await;
    support::wait_quiet(&rig).await;
    let before_tabs = rig.app.pages();
    let before_chat = rig.app.chat();
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
    // 后台轮询（列表、登录重试）本来就会来，不数总数；只找这三种报文漏出桥的样子：
    // 原样转发的报文（方法名在正文里），或它触发的动作——卸载插件的 DELETE。
    let leaked: Vec<String> = rig
        .core
        .all_seen()
        .iter()
        .filter(|seen| {
            (seen.method == Method::DELETE && seen.path.starts_with("/_wonton/api/plugins/"))
                || ["tools/call", "resources/read", "update-model-context"]
                    .iter()
                    .any(|marker| seen.body.contains(marker))
        })
        .map(|seen| format!("{} {}", seen.method, seen.path))
        .collect();
    assert!(leaked.is_empty(), "不支持的桥请求漏到了核心: {leaked:?}");
    assert_eq!(rig.app.pages(), before_tabs);
    assert_eq!(rig.app.chat(), before_chat);
}

#[tokio::test]
async fn subframes_and_notifications_have_no_request_side_effects() {
    let (rig, tab) = open_page(true).await;
    let before = rig.app.pages();
    for (method, params) in [
        ("ui/request-display-mode", json!({"mode":"fullscreen"})),
        ("ui/open-link", json!({"url":"https://example.org/"})),
        (
            "ui/message",
            json!({"role":"user","content":[{"type":"text","text":"draft"}]}),
        ),
    ] {
        let reply = rig.app.tab_handle_message(
            bar(&tab),
            false,
            PAGE_ORIGIN.to_owned(),
            json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string(),
            environment(),
        );
        assert!(reply.reply_script.is_none());
        assert!(reply.action.is_none());
        assert!(!reply.initialized);
        let notification = rig.app.tab_handle_message(
            bar(&tab),
            true,
            PAGE_ORIGIN.to_owned(),
            json!({"jsonrpc":"2.0","method":method,"params":params}).to_string(),
            environment(),
        );
        assert!(notification.action.is_none());
    }
    assert_eq!(rig.app.pages(), before);
}

#[tokio::test]
async fn display_requests_obey_both_modes_and_reject_unknown_ones() {
    let (rig, tab) = open_page(true).await;
    // 整页只有一种样子：inline 和 fullscreen 都收，状态都是 fullscreen。
    for mode in ["fullscreen", "inline"] {
        let result = request(&rig, &tab, "ui/request-display-mode", json!({"mode":mode}));
        assert_eq!(response(&result)["result"]["mode"], "fullscreen");
    }
    assert_eq!(
        response(&initialize(&rig, &tab))["result"]["hostContext"]["availableDisplayModes"],
        json!(["inline", "fullscreen"])
    );
    let before = rig.app.pages();
    assert_eq!(
        response(&request(
            &rig,
            &tab,
            "ui/request-display-mode",
            json!({"mode":"pip"})
        ))["error"]["code"],
        -32602
    );
    assert_eq!(rig.app.pages(), before);
}

#[tokio::test]
async fn unknown_modes_are_rejected_and_state_does_not_move() {
    let (rig, tab) = open_page(true).await;
    let reply = request(&rig, &tab, "ui/request-display-mode", json!({"mode":"pip"}));
    assert_eq!(response(&reply)["error"]["code"], -32602);
}
