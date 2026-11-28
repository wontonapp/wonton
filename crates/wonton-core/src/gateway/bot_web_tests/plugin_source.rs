//! 插件网络来源的请求在兑换、登录和设备身份上都受限。

use super::*;

async fn from_plugin(h: &Harness, host: &str, path: &str, cookie: &str) -> Reply {
    let mut req = hyper::Request::builder()
        .uri(path)
        .header("host", host)
        .header("cookie", cookie)
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(
        "192.0.2.10:1234".parse::<std::net::SocketAddr>().unwrap(),
    ));
    req.extensions_mut()
        .insert(TunnelPeer(iroh::SecretKey::generate().public()));
    Reply::of(h.app.clone().oneshot(req).await.unwrap()).await
}

#[tokio::test]
async fn any_plugin_source_is_denied_before_web_redemption_and_login() {
    let h = Harness::start(None).await;
    setup(&h).await;
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    let app = h.app_session_on(&host("mia")).await;
    h.manager
        .register_source_for_test("app.whoami", "192.0.2.0/24", vec![]);
    for name in ["default", "mia"] {
        for path in ["/", "/_wonton/session?code=anything&to=/"] {
            let reply = from_plugin(&h, &host(name), path, &app).await;
            assert_failure(&reply, StatusCode::FORBIDDEN, "permission_denied");
        }
    }
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("to", &h.origin(&host("mia")))
        .finish();
    for path in [
        format!("/_wonton/sso?{query}"),
        bot_web_path(&bot("mia")),
        "/_wonton/oidc/login?login_challenge=x".into(),
        "/_wonton/oidc/consent?consent_challenge=x".into(),
    ] {
        assert_failure(
            &from_plugin(&h, CORE, &path, &core).await,
            StatusCode::FORBIDDEN,
            "permission_denied",
        );
    }
}

#[tokio::test]
async fn a_plugin_cannot_login_to_core_even_with_the_owner_token() {
    let h = Harness::start(None).await;
    setup(&h).await;
    h.manager
        .register_source_for_test("app.other", "192.0.2.0/24", vec![]);
    let mut req = hyper::Request::builder()
        .method(Method::POST)
        .uri("/_wonton/login")
        .header("host", CORE)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("token={TOKEN}&next=/")))
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(
        "192.0.2.10:1234".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let reply = Reply::of(h.app.clone().oneshot(req).await.unwrap()).await;
    assert_failure(&reply, StatusCode::FORBIDDEN, "permission_denied");
    assert!(!reply.headers.contains_key(hyper::header::SET_COOKIE));
}

#[tokio::test]
async fn plugin_source_cannot_consume_an_otherwise_valid_bot_handoff() {
    let h = Harness::start(None).await;
    setup(&h).await;
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("to", &h.origin(&host("mia")))
        .finish();
    let handoff = h
        .get(CORE, &format!("/_wonton/sso?{query}"), &[("cookie", &core)])
        .await;
    let target = url::Url::parse(&handoff.location()).unwrap();
    let path = format!("{}?{}", target.path(), target.query().unwrap());
    h.manager
        .register_source_for_test("app.other", "192.0.2.0/24", vec![]);
    let denied = from_plugin(&h, &host("mia"), &path, &core).await;
    assert_failure(&denied, StatusCode::FORBIDDEN, "permission_denied");
    let redeemed = h.get(&host("mia"), &path, &[]).await;
    assert_eq!(redeemed.status, StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn a_plugin_network_source_cannot_borrow_a_paired_owner_device() {
    let h = Harness::start(None).await;
    setup(&h).await;
    let device = iroh::SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(device, None, None, |_| true)
        .unwrap();
    h.manager
        .register_source_for_test("app.other", "192.0.2.0/24", vec![]);
    let stranger = iroh::SecretKey::generate().public();
    for peer in [device, stranger] {
        for name in ["default", "mia"] {
            let response = plugin_device_request(&h, name, peer).await;
            assert_failure(&response, StatusCode::FORBIDDEN, "permission_denied");
        }
    }
}

async fn plugin_device_request(h: &Harness, name: &str, device: iroh::PublicKey) -> Reply {
    let mut request = hyper::Request::builder()
        .uri("/")
        .header("host", host(name))
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(TunnelPeer(device));
    request.extensions_mut().insert(ConnectInfo(
        "192.0.2.10:1234".parse::<std::net::SocketAddr>().unwrap(),
    ));
    Reply::of(h.app.clone().oneshot(request).await.unwrap()).await
}
