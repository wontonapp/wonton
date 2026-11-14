//! Bot 网页的目标、权限与完整主机登录边界；真实网关，独立回环上游。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::ConnectInfo;
use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wonton_protocol::{BotKeeper, BotRef, bot_web_label, bot_web_path};

use super::fakes::{echo_upstream, fake_hydra};
use super::family_tests::send_as;
use super::tests::{CORE, Harness, Reply, TOKEN, WHOAMI};
use super::{APP_COOKIE, CORE_COOKIE, TunnelPeer};
use crate::members::MemberRecord;
use crate::plugin::languages::Languages;
use crate::plugin::upstreams::Endpoint;
use crate::plugin::{PluginId, PluginPackage, PluginStatus};

fn bot(name: &str) -> BotRef {
    BotRef {
        plugin_id: "app.whoami".into(),
        bot: name.into(),
    }
}

fn host(name: &str) -> String {
    format!("{}.{}", bot_web_label(&bot(name)), CORE)
}

fn source() -> String {
    WHOAMI.replace("  icon: icon.svg\n", "")
        .replace("  permissions: []", "  permissions: [matrix.bot]\n  bots: {services: [web]}")
        .replace("    port: 8000", "    port: 8000\n    path: /panel?mode=bot\n    cross_origin_isolated: true\n    sso: oidc\n    oidc:\n      redirect_paths: [/cb]\n      admin_groups: [admin]")
}

fn record(h: &Harness, family: bool, compose: &str) {
    std::fs::write(h.data_dir.join("plugins/app.whoami.json"), json!({
        "id": "app.whoami", "version": "0.1.0", "source": compose.replace("  icon: icon.svg\n", ""),
        "granted_permissions": ["matrix.bot"], "installed_at": 1, "family": family,
    }).to_string()).unwrap();
}

async fn setup(h: &Harness) -> Arc<AtomicUsize> {
    let compose = source();
    record(h, false, &compose);
    h.manager.register_for_test(
        PluginPackage::manifest_of(&compose, &Languages::default()).unwrap(),
        PluginStatus::Running,
        Some(h.upstream.to_string()),
    );
    h.manager.bot_records_for_test(
        &PluginId::parse("app.whoami").unwrap(),
        &[
            (
                "default",
                "@whoami:test",
                BotKeeper::User {
                    username: "owner".into(),
                },
                "a",
            ),
            (
                "mia",
                "@whoami-mia:test",
                BotKeeper::User {
                    username: "mia".into(),
                },
                "b",
            ),
        ],
    );
    let other_hits = Arc::new(AtomicUsize::new(0));
    let other = echo_upstream(other_hits.clone()).await;
    h.manager
        .bot_web_route_for_test(bot("default"), Endpoint::Tcp(h.upstream.to_string()), true);
    h.manager
        .bot_web_route_for_test(bot("mia"), Endpoint::Tcp(other.to_string()), true);
    let mut registrations = serde_json::Map::new();
    for name in ["default", "mia"] {
        registrations.insert(
            bot_web_label(&bot(name)),
            json!({"bot": bot(name), "secret": "test-only"}),
        );
    }
    std::fs::create_dir_all(h.data_dir.join("plugins/bot-web")).unwrap();
    std::fs::write(
        h.data_dir.join("plugins/bot-web/registrations.json"),
        serde_json::to_string(&registrations).unwrap(),
    )
    .unwrap();
    other_hits
}

fn member(h: &Harness) -> iroh::EndpointId {
    h.members
        .invite(MemberRecord {
            username: "mia".into(),
            user_id: "@mia:test".into(),
            display_name: "Mia".into(),
            email: None,
            role_id: crate::roles::DEFAULT_INVITE_ROLE.to_owned(),
            status: wonton_protocol::MemberStatus::Joined,
            wants_private_bot: false,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        })
        .unwrap();
    let device = iroh::SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(device, None, Some("mia".into()), |_| true)
        .unwrap();
    device
}

/// #518 起可见性按角色 open_apps 算：给默认邀请角色开放这个插件，family 标记不再管事。
fn open_whoami(h: &Harness) {
    let draft = wonton_protocol::RoleDraft {
        name: "朋友".into(),
        permissions: wonton_protocol::Permissions {
            open_apps: wonton_protocol::AppScope::Selected {
                plugin_ids: vec!["app.whoami".into()],
            },
            ..wonton_protocol::Permissions::strictest()
        },
    };
    h.roles.update("friend", &draft).unwrap();
}

fn assert_failure(reply: &Reply, status: StatusCode, reason: &str) {
    assert_eq!(reply.status, status, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some(reason));
    assert!(reply.json().get("page").is_none());
}

#[tokio::test]
async fn two_bot_hosts_reach_only_their_own_upstream_and_metadata() {
    let h = Harness::start(None).await;
    let other_hits = setup(&h).await;
    let first = h.app_session_on(&host("default")).await;
    let second = h.app_session_on(&host("mia")).await;
    let a = h
        .get(&host("default"), "/first", &[("cookie", &first)])
        .await;
    let b = h.get(&host("mia"), "/second", &[("cookie", &second)]).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(b.status, StatusCode::OK);
    assert_eq!(h.hits.load(Ordering::SeqCst), 1);
    assert_eq!(other_hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        a.headers["document-isolation-policy"],
        "isolate-and-credentialless"
    );
    for name in ["default", "mia"] {
        let result = h
            .get(
                CORE,
                &bot_web_path(&bot(name)),
                &[("authorization", &format!("Bearer {TOKEN}"))],
            )
            .await;
        assert_eq!(result.status, StatusCode::OK, "{}", result.text());
        assert_eq!(
            result.json()["page"]["bot"],
            serde_json::to_value(bot(name)).unwrap()
        );
        assert_eq!(
            result.json()["page"]["url"],
            format!("{}/panel?mode=bot", h.origin(&host(name)))
        );
    }
}

#[tokio::test]
async fn family_switch_not_bot_keeper_controls_both_web_and_metadata() {
    let h = Harness::start(None).await;
    setup(&h).await;
    let device = member(&h);
    for name in ["default", "mia"] {
        for (target, path) in [
            (CORE.to_owned(), bot_web_path(&bot(name))),
            (host(name), "/".into()),
        ] {
            let reply = send_as(&h.app, Some(device), Method::GET, &target, &path, &[], "").await;
            assert_failure(&reply, StatusCode::FORBIDDEN, "not_shared");
        }
    }
    open_whoami(&h);
    for name in ["default", "mia"] {
        let reply = send_as(&h.app, Some(device), Method::GET, &host(name), "/", &[], "").await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    }
    h.tunnel.devices.remove(&device).unwrap();
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
    assert_failure(&reply, StatusCode::UNAUTHORIZED, "unauthenticated");
}

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
async fn oidc_consent_uses_exact_bot_client_and_rechecks_bot_existence() {
    let accepted = Arc::new(Mutex::new(Vec::<Value>::new()));
    let h = Harness::start(Some(fake_hydra(accepted.clone()).await)).await;
    setup(&h).await;
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    let label = bot_web_label(&bot("mia"));
    let result = h
        .get(
            CORE,
            &format!("/_wonton/oidc/consent?consent_challenge={label}"),
            &[("cookie", &core)],
        )
        .await;
    assert_eq!(result.status, StatusCode::SEE_OTHER, "{}", result.text());
    assert_eq!(
        accepted.lock().unwrap()[0]["session"]["id_token"]["groups"],
        json!(["user", "plugin_admin", "admin"])
    );
    h.manager
        .bot_records_for_test(&PluginId::parse("app.whoami").unwrap(), &[]);
    let result = h
        .get(
            CORE,
            &format!("/_wonton/oidc/consent?consent_challenge={label}"),
            &[("cookie", &core)],
        )
        .await;
    assert_failure(&result, StatusCode::NOT_FOUND, "not_found");
    assert_eq!(accepted.lock().unwrap().len(), 1);
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

async fn bot_hydra(accepted: Arc<Mutex<Vec<Value>>>) -> std::net::SocketAddr {
    use axum::extract::Query;
    use axum::routing::{get, put};
    use axum::{Json, Router};
    use std::collections::HashMap;
    let login = get(|Query(query): Query<HashMap<String, String>>| async move {
        Json(
            json!({"skip": false, "subject": "", "client": {"client_id": query["login_challenge"]}}),
        )
    });
    let accept = put(move |Json(body): Json<Value>| {
        let accepted = accepted.clone();
        async move {
            accepted.lock().unwrap().push(body);
            Json(json!({"redirect_to": "http://auth.wonton.localhost/after-login"}))
        }
    });
    let (listener, addr) = super::fakes::bind().await;
    super::fakes::serve(
        listener,
        Router::new()
            .route("/admin/oauth2/auth/requests/login", login)
            .route("/admin/oauth2/auth/requests/login/accept", accept),
    )
    .await;
    addr
}

#[tokio::test]
async fn oidc_login_checks_exact_client_family_and_target_before_accepting() {
    let accepted = Arc::new(Mutex::new(Vec::<Value>::new()));
    let h = Harness::start(Some(bot_hydra(accepted.clone()).await)).await;
    setup(&h).await;
    let device = member(&h);
    let label = bot_web_label(&bot("mia"));
    let path = format!("/_wonton/oidc/login?login_challenge={label}");
    let denied = send_as(&h.app, Some(device), Method::GET, CORE, &path, &[], "").await;
    assert_failure(&denied, StatusCode::FORBIDDEN, "not_shared");
    assert!(accepted.lock().unwrap().is_empty());
    open_whoami(&h);
    let allowed = send_as(&h.app, Some(device), Method::GET, CORE, &path, &[], "").await;
    assert_eq!(allowed.status, StatusCode::SEE_OTHER, "{}", allowed.text());
    assert_eq!(accepted.lock().unwrap()[0]["subject"], "mia");
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    h.manager
        .bot_records_for_test(&PluginId::parse("app.whoami").unwrap(), &[]);
    for client in [label, "app.whoami".into(), bot_web_label(&bot("unknown"))] {
        let reply = h
            .get(
                CORE,
                &format!("/_wonton/oidc/login?login_challenge={client}"),
                &[("cookie", &core)],
            )
            .await;
        assert_failure(&reply, StatusCode::NOT_FOUND, "not_found");
    }
    assert_eq!(accepted.lock().unwrap().len(), 1);
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
