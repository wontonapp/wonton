//! 插件的工具权限（ADR-0024）：请求发给对的接口，结果和失败原因如实带回来。

use axum::http::Method;
use serde_json::json;
use wonton_client::{CoreError, ToolAccess};

use super::rig::{Canned, Rig, core_error};

const POLICY: &str = "/_wonton/api/plugins/app.example.notes/tool-policy";
const ID: &str = "app.example.notes";

fn view(enabled: bool) -> Canned {
    let body = json!({
        "tools": [{"name": "delete", "description": null, "access": "sensitive"}],
        "tools_error": null,
        "always": [{"caller": "com.example.agent", "name": "Hermes", "enabled": enabled}],
        "allow_all": [],
        "windows": [],
        "problem": null,
    });
    Canned(200, body.to_string())
}

#[tokio::test]
async fn switches_go_to_their_own_address_and_come_back_changed() {
    let rig = Rig::new().await;
    rig.connected().await;
    let core = &rig.core;
    let id = || ID.to_owned();
    let agent = || "com.example.agent".to_owned();

    core.answer(Method::GET, POLICY, view(false));
    let policy = rig.app.tool_policy(id()).await.unwrap();
    assert_eq!(policy.tools[0].access, ToolAccess::Sensitive);
    assert!(!policy.always[0].enabled);

    let always = format!("{POLICY}/always/com.example.agent");
    core.answer(Method::PUT, &always, view(true));
    let on = rig.app.set_tool_always(id(), agent(), true).await.unwrap();
    assert!(on.always[0].enabled);
    assert_eq!(core.seen(&Method::PUT, &always).len(), 1);

    core.answer(Method::DELETE, &always, view(false));
    let off = rig.app.set_tool_always(id(), agent(), false).await.unwrap();
    assert!(!off.always[0].enabled);

    let all = format!("{POLICY}/allow-all/com.example.agent");
    core.answer(Method::PUT, &all, view(false));
    rig.app
        .set_tool_allow_all(id(), agent(), true)
        .await
        .unwrap();
    assert_eq!(core.seen(&Method::PUT, &all).len(), 1);

    let tool = format!("{POLICY}/read-only/delete");
    core.answer(Method::PUT, &tool, view(false));
    rig.app
        .set_tool_read_only(id(), "delete".to_owned(), true)
        .await
        .unwrap();
    assert_eq!(core.seen(&Method::PUT, &tool).len(), 1);
}

#[tokio::test]
async fn a_setting_that_cannot_be_saved_says_why() {
    let rig = Rig::new().await;
    rig.connected().await;
    let path = format!("{POLICY}/always/com.example.agent");
    rig.core.answer(
        Method::PUT,
        &path,
        core_error(503, "policy_unavailable", "放宽的设置读不出来"),
    );
    let failed = rig
        .app
        .set_tool_always(ID.to_owned(), "com.example.agent".to_owned(), true)
        .await;
    let Err(CoreError::Upstream { .. }) = failed else {
        panic!("应该是核心出错：{failed:?}");
    };
}

#[tokio::test]
async fn stopping_a_window_that_already_ended_is_not_a_failure() {
    let rig = Rig::new().await;
    rig.connected().await;
    let core = &rig.core;
    let window = "/_wonton/confirmations/card/window";
    core.answer(Method::GET, POLICY, view(false));
    core.answer(Method::DELETE, window, core_error(404, "not_found", "没有"));
    rig.app
        .stop_tool_window(ID.to_owned(), "card".to_owned())
        .await
        .unwrap();
    assert_eq!(core.seen(&Method::DELETE, window).len(), 1);

    core.answer(Method::DELETE, window, Canned(204, String::new()));
    core.answer(Method::GET, POLICY, view(false));
    rig.app
        .stop_tool_window(ID.to_owned(), "card".to_owned())
        .await
        .unwrap();

    core.answer(Method::DELETE, window, core_error(500, "internal", "坏了"));
    let failed = rig
        .app
        .stop_tool_window(ID.to_owned(), "card".to_owned())
        .await;
    assert!(failed.is_err());
}
