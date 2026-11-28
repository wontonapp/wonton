//! 未登录的插件请求不得到达插件。

use std::sync::atomic::Ordering;

use super::*;

#[tokio::test]
async fn unauthenticated_plugin_requests_never_reach_the_plugin() {
    let h = Harness::start(None).await;
    let page = h
        .get(PLUGIN, "/dashboard?id=1", &[("accept", "text/html")])
        .await;
    assert_eq!(page.status, StatusCode::SEE_OTHER);
    let to = format!("{}/dashboard?id=1", h.origin(PLUGIN));
    let expected = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("to", &to)
        .finish();
    assert_eq!(
        page.location(),
        format!("{}/_wonton/sso?{expected}", h.origin(CORE))
    );
    // fetch 默认的 `*/*` 和显式的 `application/json` 一样：直接 401，体是 JSON，不是登录页。
    for accept in ["*/*", "application/json"] {
        let api = h.get(PLUGIN, "/api/data", &[("accept", accept)]).await;
        assert_eq!(api.status, StatusCode::UNAUTHORIZED, "{accept}");
        assert_eq!(api.error_kind(), Some("unauthenticated"), "{accept}");
        assert_eq!(api.content_type(), "application/json", "{accept}");
        assert_eq!(
            api.json()["error"]["category"],
            "unauthenticated",
            "{accept}"
        );
        assert!(
            api.json()["error"]["message"]
                .as_str()
                .is_some_and(|m| !m.is_empty()),
            "{accept}: {}",
            api.text()
        );
    }
    // 没有 `Accept`（curl、脚本）也不给网页。
    let bare = h.get(PLUGIN, "/api/data", &[]).await;
    assert_eq!(bare.status, StatusCode::UNAUTHORIZED);
    assert_eq!(bare.json()["error"]["category"], "unauthenticated");
    // 不是导航的页面地址（不带 `Accept: text/html`）照样 401。
    let forged = h.get(PLUGIN, "/", &[("cookie", "wonton_app=forged")]).await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);
    assert_eq!(forged.json()["error"]["category"], "unauthenticated");
    assert_eq!(h.hits.load(Ordering::SeqCst), 0, "未登录的请求不得到达插件");
}
