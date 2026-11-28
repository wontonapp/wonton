//! 上游响应原样转发，只剥掉网关自己的 Cookie、替换转发头。

use std::sync::atomic::Ordering;

use serde_json::json;

use super::*;

#[tokio::test]
async fn upstream_responses_pass_through_untouched() {
    let h = Harness::start(None).await;
    let app = h.app_session().await;
    // 应用自己的 401：网关只转发，不改体，也不加 `x-wonton-error`，接口靠这个头区分是谁拦的。
    let reply = h.get(PLUGIN, "/unauthorized", &[("cookie", &app)]).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.error_kind(), None);
    assert_eq!(reply.json(), json!({"error": "app_login_required"}));

    // 正常响应也一样：头里只有上游自己的内容。
    let reply = h.get(PLUGIN, "/hello?x=1", &[("cookie", &app)]).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.error_kind(), None);
}

#[tokio::test]
async fn logged_in_requests_reach_plugin_without_gateway_cookies() {
    let h = Harness::start(None).await;
    let app = h.app_session().await;
    let cookies = format!("{app}; theme=dark; {CORE_COOKIE}=stolen");
    let headers = [("cookie", cookies.as_str()), ("x-forwarded-for", "6.6.6.6")];
    let reply = h.get(PLUGIN, "/hello?x=1", &headers).await;
    assert_eq!(reply.status, StatusCode::OK);
    let seen = reply.json();
    assert_eq!(seen["path"], "/hello?x=1");
    let seen = &seen["headers"];
    assert_eq!(seen["cookie"], "theme=dark", "网关 Cookie 必须被剥离");
    assert_eq!(seen["host"], format!("{PLUGIN}:{}", h.addr.port()));
    assert_eq!(
        seen["x-forwarded-host"],
        format!("{PLUGIN}:{}", h.addr.port())
    );
    assert_eq!(seen["x-forwarded-proto"], "http");
    assert_eq!(
        seen["x-forwarded-for"], "127.0.0.1",
        "伪造的转发头必须被替换"
    );
    assert_eq!(h.hits.load(Ordering::SeqCst), 1);

    // 插件会话只在签发它的子域名有效。
    let other = PluginPackage::manifest_of(
        &WHOAMI.replace("app.whoami", "app.other"),
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    h.manager
        .register_for_test(other, PluginStatus::Running, Some("127.0.0.1:9".into()));
    let cross = h
        .get("app-other.wonton.localhost", "/", &[("cookie", &app)])
        .await;
    assert_eq!(cross.status, StatusCode::UNAUTHORIZED);
}
