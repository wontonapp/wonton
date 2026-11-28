//! 主人令牌登录和交接码跳转只留在本服务器。

use axum::body::Body;

use super::*;

#[tokio::test]
async fn wrong_owner_token_is_rejected() {
    let h = Harness::start(None).await;
    let reply = h.login("not-the-token").await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(reply.headers.get(header::SET_COOKIE).is_none());
}

#[tokio::test]
async fn redirects_stay_on_this_server() {
    let h = Harness::start(None).await;
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    for to in [
        "http://evil.example/",
        "https://app-whoami.wonton.localhost/",
        "javascript:alert(1)",
    ] {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("to", to)
            .finish();
        let reply = h
            .get(CORE, &format!("/_wonton/sso?{query}"), &[("cookie", &core)])
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{to}");
    }
    let headers = [("content-type", "application/x-www-form-urlencoded")];
    for next in [
        "http://evil.example/",
        "/\\evil.example",
        "//evil.example",
        "javascript:alert(1)",
    ] {
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("token", TOKEN)
            .append_pair("next", next)
            .finish();
        let login = h
            .send(
                Method::POST,
                CORE,
                "/_wonton/login",
                &headers,
                Body::from(form),
            )
            .await;
        assert_eq!(login.location(), "/", "{next}");
    }

    // 交接码兑换后的跳转也只能留在插件自己的域名里。
    let to = format!("{}/", h.origin(PLUGIN));
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("to", &to)
        .finish();
    let handoff = h
        .get(CORE, &format!("/_wonton/sso?{query}"), &[("cookie", &core)])
        .await;
    let target = url::Url::parse(&handoff.location()).unwrap();
    let code = target
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .unwrap();
    let crafted = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("code", &code)
        .append_pair("to", "/\\evil.example")
        .finish();
    let redeemed = h
        .get(PLUGIN, &format!("/_wonton/session?{crafted}"), &[])
        .await;
    assert_eq!(redeemed.status, StatusCode::SEE_OTHER);
    assert_eq!(redeemed.location(), "/");
}
