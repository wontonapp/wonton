//! 会话、交接码、Cookie 和兄弟来源都只认完整主机。

use super::*;

#[tokio::test]
async fn bot_sessions_handoffs_and_upstream_cookies_are_full_host_only() {
    let h = Harness::start(None).await;
    setup(&h).await;
    let cookie = h.app_session_on(&host("mia")).await;
    let wrong = h.get(&host("default"), "/", &[("cookie", &cookie)]).await;
    assert_failure(&wrong, StatusCode::UNAUTHORIZED, "unauthenticated");
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .append_pair(
            "c",
            "app_session=abc; Domain=.wonton.localhost; Path=/; HttpOnly",
        )
        .finish();
    let reply = h
        .get(
            &host("mia"),
            &format!("/set-cookie?{encoded}"),
            &[("cookie", &cookie)],
        )
        .await;
    assert_eq!(
        reply.headers[hyper::header::SET_COOKIE],
        "app_session=abc; Path=/; HttpOnly"
    );
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("to", &format!("{}/cb", h.origin(&host("mia"))))
        .finish();
    let handoff = h
        .get(CORE, &format!("/_wonton/sso?{query}"), &[("cookie", &core)])
        .await;
    let target = url::Url::parse(&handoff.location()).unwrap();
    assert_eq!(target.host_str(), Some(host("mia").as_str()));
    let path = format!("{}?{}", target.path(), target.query().unwrap());
    assert_eq!(
        h.get(&host("default"), &path, &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.get(&host("mia"), &path, &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
    let handoff = h
        .get(CORE, &format!("/_wonton/sso?{query}"), &[("cookie", &core)])
        .await;
    let target = url::Url::parse(&handoff.location()).unwrap();
    let path = format!("{}?{}", target.path(), target.query().unwrap());
    let redeemed = h.get(&host("mia"), &path, &[]).await;
    assert_eq!(redeemed.status, StatusCode::SEE_OTHER);
    let set = redeemed.headers[hyper::header::SET_COOKIE]
        .to_str()
        .unwrap();
    assert!(
        set.starts_with(&format!("{APP_COOKIE}=")) && !set.to_ascii_lowercase().contains("domain=")
    );
}

#[tokio::test]
async fn bot_resources_and_websocket_requests_do_not_accept_sibling_origin_credentials() {
    let h = Harness::start(None).await;
    let other_hits = setup(&h).await;
    let cookie = h.app_session_on(&host("mia")).await;
    let sibling = h.origin(&host("default"));
    for path in ["/media/audio", "/ws", "/document"] {
        let reply = h
            .get(
                &host("mia"),
                path,
                &[
                    ("cookie", &cookie),
                    ("origin", &sibling),
                    ("sec-fetch-site", "same-site"),
                ],
            )
            .await;
        assert_failure(&reply, StatusCode::UNAUTHORIZED, "unauthenticated");
    }
    assert_eq!(other_hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn owner_device_can_open_and_revoked_device_cannot_borrow_a_bot_cookie() {
    let h = Harness::start(None).await;
    setup(&h).await;
    let device = iroh::SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(device, None, None, |_| true)
        .unwrap();
    let cookie = h.app_session_on(&host("mia")).await;
    let opened = send_as(
        &h.app,
        Some(device),
        Method::GET,
        &host("mia"),
        "/",
        &[],
        "",
    )
    .await;
    assert_eq!(opened.status, StatusCode::OK);
    let metadata = send_as(
        &h.app,
        Some(device),
        Method::GET,
        CORE,
        &bot_web_path(&bot("mia")),
        &[],
        "",
    )
    .await;
    assert_eq!(metadata.status, StatusCode::OK);
    h.tunnel.devices.remove(&device).unwrap();
    let denied = send_as(
        &h.app,
        Some(device),
        Method::GET,
        &host("mia"),
        "/",
        &[("cookie", &cookie)],
        "",
    )
    .await;
    assert_failure(&denied, StatusCode::UNAUTHORIZED, "unauthenticated");
}
