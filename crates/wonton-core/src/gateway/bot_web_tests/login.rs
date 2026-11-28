//! 匿名导航与已移除成员的登录跳转。

use super::*;

/// 外部浏览器没有身份时要被送去登录（GET 且要网页），接口式请求留在 401；Bot 网页和普通插件网页同款。
#[tokio::test]
async fn anonymous_browser_navigation_is_sent_to_login_but_api_requests_stay_401() {
    let h = Harness::start(None).await;
    setup(&h).await;
    let navigation = h
        .get(
            &host("default"),
            "/panel?mode=bot",
            &[("accept", "text/html")],
        )
        .await;
    assert_eq!(
        navigation.status,
        StatusCode::SEE_OTHER,
        "{}",
        navigation.text()
    );
    assert_eq!(
        navigation.location(),
        sso_location(&h, &host("default"), "/panel?mode=bot")
    );
    for accept in ["*/*", "application/json"] {
        let api = h
            .get(&host("default"), "/panel", &[("accept", accept)])
            .await;
        assert_eq!(api.status, StatusCode::UNAUTHORIZED, "{accept}");
        assert_eq!(api.error_kind(), Some("unauthenticated"), "{accept}");
        assert_eq!(api.content_type(), "application/json", "{accept}");
    }
    // 表单提交不是导航：要网页也不跳，仍是 401。
    let post = h
        .send(
            Method::POST,
            &host("default"),
            "/submit",
            &[("accept", "text/html")],
            Body::from("x=1"),
        )
        .await;
    assert_eq!(post.status, StatusCode::UNAUTHORIZED);
    assert_eq!(post.error_kind(), Some("unauthenticated"));
    assert_eq!(h.hits.load(Ordering::SeqCst), 0, "未登录的请求不得到达上游");
    // 浏览器带着这个域的 APP Cookie 回来就照常放行。
    let cookie = h.app_session_on(&host("default")).await;
    let page = h
        .get(
            &host("default"),
            "/panel",
            &[("cookie", &cookie), ("accept", "text/html")],
        )
        .await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text());
}

/// 拿一个家人自己这个域名的 APP Cookie：设备身份走完整交接，浏览器换成 Cookie 再继续。
async fn member_session(h: &Harness, device: iroh::EndpointId, host: &str) -> String {
    let to = format!("{}/", h.origin(host));
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("to", &to)
        .finish();
    let handoff = send_as(
        &h.app,
        Some(device),
        Method::GET,
        CORE,
        &format!("/_wonton/sso?{query}"),
        &[],
        "",
    )
    .await;
    assert_eq!(handoff.status, StatusCode::SEE_OTHER, "{}", handoff.text());
    let target = url::Url::parse(&handoff.location()).unwrap();
    let path = format!("{}?{}", target.path(), target.query().unwrap());
    let redeemed = h.get(host, &path, &[]).await;
    assert_eq!(
        redeemed.status,
        StatusCode::SEE_OTHER,
        "{}",
        redeemed.text()
    );
    redeemed.cookie(APP_COOKIE)
}

/// 这个域名上某个路径该跳到的 SSO 地址。
fn sso_location(h: &Harness, host: &str, path: &str) -> String {
    let to = format!("{}{path}", h.origin(host));
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("to", &to)
        .finish();
    format!("{}/_wonton/sso?{query}", h.origin(CORE))
}

/// 名单里已经移除、Cookie 还没过期的人：和匿名浏览器一样送回登录。
#[tokio::test]
async fn a_removed_member_with_a_live_cookie_is_sent_to_login() {
    let h = Harness::start(None).await;
    setup(&h).await;
    let device = member(&h);
    open_whoami(&h);
    let cookie = member_session(&h, device, &host("mia")).await;
    h.members.remove("mia", false, false).unwrap();
    let reply = h
        .get(
            &host("mia"),
            "/panel",
            &[("cookie", &cookie), ("accept", "text/html")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.text());
    assert_eq!(reply.location(), sso_location(&h, &host("mia"), "/panel"));
}

/// 核心会话还在（Cookie 没删）、人已经移除：SSO 入口重新走登录，不签发交接码。
#[tokio::test]
async fn sso_restarts_login_for_a_removed_member() {
    let h = Harness::start(None).await;
    setup(&h).await;
    member(&h);
    open_whoami(&h);
    let sid = h
        .gateway
        .sessions
        .create_core(&super::Person::member("mia"));
    let cookie = format!("{CORE_COOKIE}={sid}");
    let sso = sso_location(&h, &host("mia"), "/panel");
    let path = sso.split_once(&h.origin(CORE)).unwrap().1.to_owned();
    let active = h.get(CORE, &path, &[("cookie", &cookie)]).await;
    assert_eq!(active.status, StatusCode::SEE_OTHER, "{}", active.text());
    assert!(active.location().contains("/_wonton/session?"), "{sso}");
    h.members.remove("mia", false, false).unwrap();
    let reply = h.get(CORE, &path, &[("cookie", &cookie)]).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.text());
    let next = format!(
        "{}/_wonton/sso?{}",
        h.origin(CORE),
        path.split_once('?').unwrap().1
    );
    let expected = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("next", &next)
        .finish();
    assert_eq!(reply.location(), format!("/_wonton/login?{expected}"));
}

/// 交接码签发后、兑换前被移除：兑换处送回 SSO，由它重新走登录。
#[tokio::test]
async fn redeem_sends_a_removed_member_back_through_sso() {
    let h = Harness::start(None).await;
    setup(&h).await;
    member(&h);
    open_whoami(&h);
    let code = h
        .gateway
        .sessions
        .create_code(&super::Person::member("mia"), &host("mia"));
    h.members.remove("mia", false, false).unwrap();
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("code", &code)
        .append_pair("to", "/panel")
        .finish();
    let reply = h
        .get(
            &host("mia"),
            &format!("/_wonton/session?{query}"),
            &[("accept", "text/html")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.text());
    assert_eq!(reply.location(), sso_location(&h, &host("mia"), "/panel"));
}
