//! OIDC 回调的网关访问边界；应用自己的 state、关联 Cookie 和 nonce 校验由真实 OIDC 栈验证。

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use axum::body::Body;
use hyper::header;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::json;

use super::APP_COOKIE;
use super::family_tests::{bearer, send_as};
use super::tests::{CORE, Harness, PLUGIN, Reply, WHOAMI};
use crate::members::MemberRecord;
use crate::plugin::languages::Languages;
use crate::plugin::proxy_auth::ProxyAuth;
use crate::plugin::{PluginPackage, PluginStatus};

const CALLBACK_HOST: &str = "app-callback.wonton.localhost";
const OTHER_HOST: &str = "app-other.wonton.localhost";
const PROXY_HOST: &str = "app-proxy.wonton.localhost";
const AUTH_HOST: &str = "auth.wonton.localhost";
const CALLBACK: &str = "/signin-oidc";
const FORM: &str = "application/x-www-form-urlencoded";
const OIDC_SOURCE: &str = "\
services:
  web:
    image: traefik/whoami:v1.11
x-wonton:
  spec: '0.4'
  id: app.callback
  version: 0.1.0
  kind: app
  title: {en: Callback}
  description: {en: Callback access fixture}
  icon: icon.svg
  webui:
    service: web
    port: 8000
    sso: oidc
    oidc:
      redirect_paths: [/signin-oidc, /alternate-callback]
      post_logout_paths: [/signed-out]
  permissions: []
";

async fn fixture() -> Harness {
    let h = Harness::start(None).await;
    register(&h, OIDC_SOURCE);
    register(
        &h,
        &OIDC_SOURCE
            .replace("app.callback", "app.other")
            .replace("[/signin-oidc, /alternate-callback]", "[/other-callback]"),
    );
    register(
        &h,
        &WHOAMI.replace("app.whoami", "app.proxy").replace(
            "    port: 8000",
            "    port: 8000\n    sso: proxy\n    proxy:\n      headers: {Remote-User: '{user}'}",
        ),
    );
    // 家人开关的公开 API 更新磁盘记录；register_for_test 只填路由表。
    std::fs::write(
        h.data_dir.join("plugins/app.callback.json"),
        json!({
            "id": "app.callback",
            "version": "0.1.0",
            "source": OIDC_SOURCE,
            "granted_permissions": [],
            "installed_at": 1,
            "family": false,
        })
        .to_string(),
    )
    .unwrap();
    h
}

fn register(h: &Harness, source: &str) {
    let langs = Languages::from_header(Some("en"));
    let manifest = PluginPackage::manifest_of(source, &langs).unwrap();
    let auth = ProxyAuth::resolve(&manifest, &BTreeMap::new(), &langs).unwrap();
    let id = manifest.id.clone();
    h.manager.register_for_test(
        manifest,
        PluginStatus::Running,
        Some(h.upstream.to_string()),
    );
    if let Some(auth) = auth {
        h.manager.proxy_for_test(&id, auth);
    }
}

async fn post(h: &Harness, host: &str, path: &str, headers: &[(&str, &str)]) -> Reply {
    h.send(
        Method::POST,
        host,
        path,
        headers,
        Body::from("code=fixture-code&state=fixture-state"),
    )
    .await
}

fn assert_blocked(h: &Harness, before: usize, reply: &Reply, kind: &str) {
    let status = if kind == "not_shared" {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::UNAUTHORIZED
    };
    assert_eq!(reply.status, status, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some(kind));
    assert!(!reply.headers.contains_key(header::LOCATION));
    assert_eq!(h.hits.load(Ordering::SeqCst), before);
}

fn assert_reachable(h: &Harness, before: usize, reply: &Reply) {
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.error_kind(), None);
    assert_eq!(h.hits.load(Ordering::SeqCst), before + 1);
    assert!(!reply.headers.contains_key(header::SET_COOKIE));
}

#[tokio::test]
async fn a_form_callback_without_a_session_does_not_authorize_other_requests() {
    let h = fixture().await;
    let auth = h.origin(AUTH_HOST);
    let headers = [
        ("content-type", FORM),
        ("origin", auth.as_str()),
        ("sec-fetch-site", "same-site"),
        ("sec-fetch-mode", "navigate"),
        ("sec-fetch-dest", "document"),
    ];
    let before = h.hits.load(Ordering::SeqCst);
    let callback = post(&h, CALLBACK_HOST, CALLBACK, &headers).await;
    assert_reachable(&h, before, &callback);

    for (host, path) in [
        (CALLBACK_HOST, "/notes"),
        (PLUGIN, CALLBACK),
        (PROXY_HOST, CALLBACK),
        (OTHER_HOST, CALLBACK),
    ] {
        let before = h.hits.load(Ordering::SeqCst);
        let reply = post(&h, host, path, &headers).await;
        assert_blocked(&h, before, &reply, "unauthenticated");
    }
    // 没有网关凭据的回调不能为下一次请求建立会话。
    let before = h.hits.load(Ordering::SeqCst);
    let reply = h.get(CALLBACK_HOST, "/notes", &[]).await;
    assert_blocked(&h, before, &reply, "unauthenticated");
}

#[tokio::test]
async fn callback_paths_are_exact_and_scoped_to_the_declaring_plugin() {
    let h = fixture().await;
    let auth = h.origin(AUTH_HOST);
    let headers = [("content-type", FORM), ("origin", auth.as_str())];
    for path in [
        "/signin-oidc/",
        "/signin-oidc/notes",
        "/signin-oidc-suffix",
        "/signin",
        "/Signin-oidc",
        "/%73ignin-oidc",
        "/signin%2Doidc",
        "/signed-out",
        "/other-callback",
    ] {
        let before = h.hits.load(Ordering::SeqCst);
        let reply = post(&h, CALLBACK_HOST, path, &headers).await;
        assert_blocked(&h, before, &reply, "unauthenticated");
    }
    for (host, path) in [
        (CALLBACK_HOST, CALLBACK),
        (CALLBACK_HOST, "/alternate-callback"),
        (CALLBACK_HOST, "/signin-oidc?return=%2Fnotes"),
        (OTHER_HOST, "/other-callback"),
    ] {
        let before = h.hits.load(Ordering::SeqCst);
        let reply = post(&h, host, path, &headers).await;
        assert_reachable(&h, before, &reply);
    }
}

#[tokio::test]
async fn only_real_urlencoded_form_media_types_open_the_callback() {
    let h = fixture().await;
    let auth = h.origin(AUTH_HOST);
    for (content_type, allowed) in [
        (Some(FORM), true),
        (
            Some("application/x-www-form-urlencoded; charset=UTF-8"),
            true,
        ),
        (Some("Application/X-WWW-Form-Urlencoded"), true),
        (
            Some(" application/x-www-form-urlencoded ; charset=UTF-8"),
            true,
        ),
        (Some("application/json"), false),
        (Some("text/plain"), false),
        (Some("multipart/form-data; boundary=fixture"), false),
        (Some("application/x-www-form-urlencodedevil"), false),
        (Some("application/x-www-form-urlencoded+json"), false),
        (None, false),
    ] {
        let mut headers = vec![("origin", auth.as_str()), ("sec-fetch-site", "same-site")];
        if let Some(content_type) = content_type {
            headers.push(("content-type", content_type));
        }
        let before = h.hits.load(Ordering::SeqCst);
        let reply = post(&h, CALLBACK_HOST, CALLBACK, &headers).await;
        if allowed {
            assert_reachable(&h, before, &reply);
        } else {
            assert_blocked(&h, before, &reply, "unauthenticated");
        }
    }
}

#[tokio::test]
async fn callback_origins_are_restricted_even_without_fetch_metadata() {
    let h = fixture().await;
    let auth = h.origin(AUTH_HOST);
    let own = h.origin(CALLBACK_HOST);
    let other = h.origin(OTHER_HOST);
    let core = h.origin(CORE);
    let upper_auth = auth.to_ascii_uppercase();
    let upper_own = own.to_ascii_uppercase();
    let wrong_auth_port = format!("http://{AUTH_HOST}:{}", h.addr.port() ^ 1);
    let wrong_own_port = format!("http://{CALLBACK_HOST}:{}", h.addr.port() ^ 1);
    for site in [Some("same-site"), Some("cross-site"), None] {
        for (origin, allowed) in [
            (Some(auth.as_str()), true),
            (Some(own.as_str()), true),
            (Some(upper_auth.as_str()), true),
            (Some(upper_own.as_str()), true),
            (None, true),
            (Some("http://evil.example"), false),
            (Some("null"), false),
            (Some(other.as_str()), false),
            (Some(core.as_str()), false),
            (Some(wrong_auth_port.as_str()), false),
            (Some(wrong_own_port.as_str()), false),
        ] {
            let mut headers = vec![("content-type", FORM)];
            if let Some(site) = site {
                headers.extend([
                    ("sec-fetch-site", site),
                    ("sec-fetch-mode", "navigate"),
                    ("sec-fetch-dest", "document"),
                ]);
            }
            if let Some(origin) = origin {
                headers.push(("origin", origin));
            }
            let before = h.hits.load(Ordering::SeqCst);
            let reply = post(&h, CALLBACK_HOST, CALLBACK, &headers).await;
            if allowed {
                assert_reachable(&h, before, &reply);
            } else {
                assert_blocked(&h, before, &reply, "unauthenticated");
            }
        }
    }
}

#[tokio::test]
async fn non_post_callbacks_still_require_a_gateway_session() {
    let h = fixture().await;
    let auth = h.origin(AUTH_HOST);
    let headers = [
        ("content-type", FORM),
        ("origin", auth.as_str()),
        ("sec-fetch-site", "same-site"),
        ("sec-fetch-mode", "navigate"),
        ("sec-fetch-dest", "document"),
    ];
    for method in [
        Method::GET,
        Method::HEAD,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
    ] {
        let before = h.hits.load(Ordering::SeqCst);
        let reply = h
            .send(method, CALLBACK_HOST, CALLBACK, &headers, Body::empty())
            .await;
        assert_blocked(&h, before, &reply, "unauthenticated");
    }
}

#[tokio::test]
async fn query_callbacks_keep_the_html_handoff_and_session_requirement() {
    let h = fixture().await;
    let path = "/signin-oidc?code=query-code&state=query-state";
    let navigation = [
        ("accept", "text/html"),
        ("sec-fetch-site", "same-site"),
        ("sec-fetch-mode", "navigate"),
        ("sec-fetch-dest", "document"),
    ];
    let before = h.hits.load(Ordering::SeqCst);
    let redirect = h.get(CALLBACK_HOST, path, &navigation).await;
    assert_eq!(redirect.status, StatusCode::SEE_OTHER);
    let target = url::Url::parse(&redirect.location()).unwrap();
    assert_eq!(target.origin().ascii_serialization(), h.origin(CORE));
    assert_eq!(target.path(), "/_wonton/sso");
    let to = target
        .query_pairs()
        .find(|(key, _)| key == "to")
        .unwrap()
        .1
        .into_owned();
    assert_eq!(to, format!("{}{path}", h.origin(CALLBACK_HOST)));
    assert_eq!(h.hits.load(Ordering::SeqCst), before);

    let anonymous = h.get(CALLBACK_HOST, path, &[]).await;
    assert_blocked(&h, before, &anonymous, "unauthenticated");
    let session = h.app_session_on(CALLBACK_HOST).await;
    let mut headers = navigation.to_vec();
    headers.push(("cookie", session.as_str()));
    let before = h.hits.load(Ordering::SeqCst);
    let authenticated = h.get(CALLBACK_HOST, path, &headers).await;
    assert_reachable(&h, before, &authenticated);
}

fn member_device(h: &Harness) -> iroh::EndpointId {
    h.members
        .invite(MemberRecord {
            username: "mia".into(),
            user_id: "@mia:test".into(),
            display_name: "Mia".into(),
            email: None,
            status: wonton_protocol::MemberStatus::Invited,
            role_id: "friend".to_owned(),
            can_install_plugins: false,
            wants_private_bot: false,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        })
        .unwrap();
    let device = SecretKey::from_bytes(&[42; 32]).public();
    h.tunnel
        .devices
        .add_if(device, None, Some("mia".into()), |_| true)
        .unwrap();
    device
}

async fn member_session(h: &Harness, device: iroh::EndpointId) -> String {
    let to = format!("{}{CALLBACK}", h.origin(CALLBACK_HOST));
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
    assert_eq!(handoff.status, StatusCode::SEE_OTHER);
    let target = url::Url::parse(&handoff.location()).unwrap();
    let redeemed = h
        .get(
            CALLBACK_HOST,
            &format!("{}?{}", target.path(), target.query().unwrap()),
            &[],
        )
        .await;
    assert_eq!(redeemed.status, StatusCode::SEE_OTHER);
    redeemed.cookie(APP_COOKIE)
}

async fn set_family(h: &Harness, enabled: bool) {
    let token = bearer();
    let reply = h
        .send(
            Method::PUT,
            CORE,
            "/_wonton/api/plugins/app.callback/family",
            &[
                ("authorization", token.as_str()),
                ("content-type", "application/json"),
            ],
            Body::from(json!({"enabled": enabled}).to_string()),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.json()["family"], enabled);
}

#[tokio::test]
async fn recognized_members_keep_the_family_gate_on_callback_posts() {
    let h = fixture().await;
    let device = member_device(&h);
    let session = member_session(&h, device).await;
    let auth = h.origin(AUTH_HOST);
    let headers = [
        ("content-type", FORM),
        ("origin", auth.as_str()),
        ("sec-fetch-site", "same-site"),
        ("sec-fetch-mode", "navigate"),
    ];
    for enabled in [false, true, false] {
        set_family(&h, enabled).await;
        for via_tunnel in [false, true] {
            let mut request_headers = headers.to_vec();
            if !via_tunnel {
                request_headers.push(("cookie", session.as_str()));
            }
            let before = h.hits.load(Ordering::SeqCst);
            let reply = if via_tunnel {
                send_as(
                    &h.app,
                    Some(device),
                    Method::POST,
                    CALLBACK_HOST,
                    CALLBACK,
                    &request_headers,
                    "code=fixture-code&state=fixture-state",
                )
                .await
            } else {
                post(&h, CALLBACK_HOST, CALLBACK, &request_headers).await
            };
            if enabled {
                assert_reachable(&h, before, &reply);
            } else {
                assert_blocked(&h, before, &reply, "not_shared");
            }
        }
        // 无身份回调不推测家人身份；它仍不能授权后续的普通网页请求。
        let before = h.hits.load(Ordering::SeqCst);
        let anonymous = post(&h, CALLBACK_HOST, CALLBACK, &headers).await;
        assert_reachable(&h, before, &anonymous);
    }
}

#[tokio::test]
async fn an_owner_session_on_a_form_callback_does_not_leak_gateway_cookies() {
    let h = fixture().await;
    let session = h.app_session_on(CALLBACK_HOST).await;
    let auth = h.origin(AUTH_HOST);
    let cookies = format!("{session}; wonton_session=foreign; app_correlation=fixture");
    let before = h.hits.load(Ordering::SeqCst);
    let reply = post(
        &h,
        CALLBACK_HOST,
        CALLBACK,
        &[
            ("content-type", FORM),
            ("origin", auth.as_str()),
            ("sec-fetch-site", "same-site"),
            ("cookie", cookies.as_str()),
        ],
    )
    .await;
    assert_reachable(&h, before, &reply);
    assert_eq!(reply.json()["headers"]["cookie"], "app_correlation=fixture");
}

#[tokio::test]
async fn existing_sessions_do_not_expand_the_cross_origin_callback_exception() {
    let h = fixture().await;
    let auth = h.origin(AUTH_HOST);
    for (host, path, content_type, origin) in [
        (CALLBACK_HOST, "/notes", FORM, auth.as_str()),
        (CALLBACK_HOST, CALLBACK, "application/json", auth.as_str()),
        (CALLBACK_HOST, CALLBACK, FORM, "http://evil.example"),
        (CALLBACK_HOST, CALLBACK, FORM, "null"),
        (CALLBACK_HOST, "/signin-oidc/", FORM, auth.as_str()),
        (PLUGIN, CALLBACK, FORM, auth.as_str()),
        (PROXY_HOST, CALLBACK, FORM, auth.as_str()),
        (OTHER_HOST, CALLBACK, FORM, auth.as_str()),
    ] {
        let session = h.app_session_on(host).await;
        let before = h.hits.load(Ordering::SeqCst);
        let reply = post(
            &h,
            host,
            path,
            &[
                ("cookie", session.as_str()),
                ("content-type", content_type),
                ("origin", origin),
                ("sec-fetch-site", "same-site"),
                ("sec-fetch-mode", "navigate"),
            ],
        )
        .await;
        assert_blocked(&h, before, &reply, "unauthenticated");
    }
}
