//! 元数据接口的 JSON 契约与存储失败。

use super::*;

#[tokio::test]
async fn metadata_storage_failure_is_not_not_found_or_not_shared() {
    let h = Harness::start(None).await;
    setup(&h).await;
    let device = member(&h);
    open_whoami(&h);
    std::fs::write(h.data_dir.join("plugins/app.whoami.json"), "{").unwrap();
    let reply = send_as(
        &h.app,
        Some(device),
        Method::GET,
        CORE,
        &bot_web_path(&bot("mia")),
        &[],
        "",
    )
    .await;
    assert_failure(&reply, StatusCode::INTERNAL_SERVER_ERROR, "storage");
    assert_eq!(reply.json()["reason"], "other");
    let body = reply.json();
    assert!(
        body["error"]["detail"]
            .as_str()
            .unwrap()
            .contains(h.data_dir.join("plugins/app.whoami.json").to_str().unwrap())
    );
}

/// 元数据接口按契约一直是 JSON，浏览器打开也不跳。
#[tokio::test]
async fn the_bot_web_metadata_api_stays_json_for_browser_navigation() {
    let h = Harness::start(None).await;
    setup(&h).await;
    let reply = h
        .get(
            CORE,
            &bot_web_path(&bot("default")),
            &[("accept", "text/html")],
        )
        .await;
    assert_failure(&reply, StatusCode::UNAUTHORIZED, "unauthenticated");
    assert_eq!(reply.content_type(), "application/json");
}
