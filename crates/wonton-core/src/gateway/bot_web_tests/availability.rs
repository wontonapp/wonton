//! 缺失、未就绪和没有网页都不落回默认 Bot。

use super::*;

#[tokio::test]
async fn missing_unready_and_no_bot_web_never_fall_back_to_default() {
    let h = Harness::start(None).await;
    setup(&h).await;
    let auth = format!("Bearer {TOKEN}");
    let headers = [("authorization", auth.as_str())];
    let missing = h.get(CORE, &bot_web_path(&bot("absent")), &headers).await;
    assert_failure(&missing, StatusCode::NOT_FOUND, "not_found");
    h.manager
        .bot_web_route_for_test(bot("mia"), Endpoint::Tcp(h.upstream.to_string()), false);
    let unready = h.get(CORE, &bot_web_path(&bot("mia")), &headers).await;
    assert_failure(&unready, StatusCode::SERVICE_UNAVAILABLE, "not_ready");
    record(&h, false, WHOAMI);
    let no_web = h.get(CORE, &bot_web_path(&bot("default")), &headers).await;
    assert_failure(&no_web, StatusCode::NOT_FOUND, "no_webui");
    let unknown = h.get(&host("absent"), "/", &[]).await;
    assert_failure(&unknown, StatusCode::NOT_FOUND, "not_found");
    assert_eq!(h.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn sso_does_not_issue_handoffs_for_unshared_or_unready_bots() {
    let h = Harness::start(None).await;
    setup(&h).await;
    let device = member(&h);
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("to", &h.origin(&host("mia")))
        .finish();
    let path = format!("/_wonton/sso?{query}");
    let denied = send_as(&h.app, Some(device), Method::GET, CORE, &path, &[], "").await;
    assert_failure(&denied, StatusCode::FORBIDDEN, "not_shared");
    open_whoami(&h);
    h.manager
        .bot_web_route_for_test(bot("mia"), Endpoint::Tcp(h.upstream.to_string()), false);
    let unready = send_as(&h.app, Some(device), Method::GET, CORE, &path, &[], "").await;
    assert_failure(&unready, StatusCode::SERVICE_UNAVAILABLE, "not_ready");
    assert!(!unready.headers.contains_key(hyper::header::LOCATION));
}
