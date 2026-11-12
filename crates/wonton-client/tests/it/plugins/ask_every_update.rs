//! 详情页的“每次都问”开关（ADR-0033）：请求发给对的接口，返回的详情带着新值。

use axum::http::Method;
use wonton_client::{PluginDetail, PluginKind, PluginTrust};

use super::rig::{Canned, Rig, core_error};

const ID: &str = "app.agent.notes";
const PATH: &str = "/_wonton/api/plugins/app.agent.notes/ask-every-update";

fn detail(ask: bool) -> Canned {
    let detail = PluginDetail {
        source: None,
        id: ID.to_owned(),
        title: "标题 app.agent.notes".to_owned(),
        summary: String::new(),
        about: None,
        icon: None,
        screenshots: Vec::new(),
        tags: Vec::new(),
        kind: PluginKind::App,
        version: "1.0.0".to_owned(),
        trust: PluginTrust::Official,
        provider: None,
        via_index: None,
        author: None,
        homepage: None,
        repository: None,
        license: None,
        stars: None,
        release_notes: None,
        permissions: Vec::new(),
        requirements: Vec::new(),
        secret_inputs: Vec::new(),
        has_web: true,
        installed: None,
        blocked: None,
        notices: Vec::new(),
        missing_requires: Vec::new(),
        missing_wants: Vec::new(),
        hints: Vec::new(),
        capability_providers: Vec::new(),
        lan_webui: None,
        save_grants: Vec::new(),
        limits: None,
        ask_every_update: Some(ask),
        shares: None,
    };
    Canned(200, serde_json::to_string(&detail).unwrap())
}

#[tokio::test]
async fn the_switch_reaches_the_core_and_the_detail_comes_back_changed() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.answer(Method::PUT, PATH, detail(true));
    let on = rig
        .app
        .set_ask_every_update(ID.to_owned(), true)
        .await
        .unwrap();
    assert_eq!(on.ask_every_update, Some(true));
    let seen = &rig.core.seen(&Method::PUT, PATH)[0];
    assert_eq!(seen.body, r#"{"enabled":true}"#);

    rig.core.answer(Method::PUT, PATH, detail(false));
    let off = rig
        .app
        .set_ask_every_update(ID.to_owned(), false)
        .await
        .unwrap();
    assert_eq!(off.ask_every_update, Some(false));
    assert_eq!(rig.core.seen(&Method::PUT, PATH).len(), 2);
}

#[tokio::test]
async fn a_refused_switch_reports_the_reason_of_the_core() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.answer(
        Method::PUT,
        PATH,
        core_error(400, "invalid_input", "这个插件不是 Agent 写的"),
    );
    let result = rig.app.set_ask_every_update(ID.to_owned(), true).await;
    assert!(
        matches!(&result, Err(wonton_client::CoreError::InvalidInput { reason }) if reason == "这个插件不是 Agent 写的"),
        "{result:?}"
    );
    let seen = &rig.core.seen(&Method::PUT, PATH)[0];
    assert_eq!(seen.body, r#"{"enabled":true}"#);
}
