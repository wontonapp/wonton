use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Query, State},
};
use hyper::{Method, StatusCode};
use serde_json::json;
use wonton_protocol::{ShareGrantUpdate, ShareRecipient};

use super::*;
use crate::gateway::{
    sso,
    tests::{Harness, TOKEN, owner},
};
use crate::{friends::Friends, shares::Shares};

fn peer() -> SharePeer {
    SharePeer {
        user_id: "@bob:example.org".into(),
        generation: 7,
    }
}

fn permits(generation: u64) -> Vec<crate::shares::GrantPermit> {
    vec![crate::shares::GrantPermit {
        grant_id: "share-1".into(),
        generation,
        group_generation: None,
    }]
}

#[test]
fn auth_surface_is_exact_and_code_only() {
    for path in [crate::oidc::DISCOVERY_PATH, "/.well-known/jwks.json"] {
        assert_eq!(
            auth_route(&Method::GET, &path.parse().unwrap()),
            Ok(AuthRoute::Metadata)
        );
        assert!(auth_route(&Method::POST, &path.parse().unwrap()).is_err());
    }
    assert!(matches!(
        auth_route(
            &Method::GET,
            &"/oauth2/auth?client_id=app.game&response_type=code"
                .parse()
                .unwrap()
        ),
        Ok(AuthRoute::Authorize { .. })
    ));
    for path in [
        "/oauth2/token",
        "/oauth2/revoke",
        "/userinfo",
        "/oauth2/introspect",
        "/oauth2/sessions/logout",
        "/admin/clients",
        "/oauth2/auth/",
        "/oauth2/fallbacks/login",
    ] {
        assert!(
            auth_route(&Method::GET, &path.parse().unwrap()).is_err(),
            "{path}"
        );
    }
    for query in [
        "",
        "client_id=",
        "client_id=app.game",
        "client_id=app.game&response_type=token",
        "client_id=app.game&response_type=id_token",
        "client_id=app.game&response_type=code&client_id=app.other",
        "client_id=app.game&response_type=code&%63lient_id=app.other",
        "client_id=app.game&response_type=code&request=x",
        "client_id=app.game&response_type=code&request_uri=x",
        "client_id=app.game&login_verifier=",
        "client_id=app.game&login_verifier=a&consent_verifier=b",
        "client_id=app.game&device_verifier=a",
    ] {
        let uri = format!("/oauth2/auth?{query}").parse().unwrap();
        assert!(auth_route(&Method::GET, &uri).is_err(), "{query}");
    }
    let uri = "/oauth2/auth?client_id=app.game&response_type=code"
        .parse()
        .unwrap();
    for method in [
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::HEAD,
        Method::OPTIONS,
    ] {
        assert!(auth_route(&method, &uri).is_err());
    }
}

#[test]
fn verifier_capabilities_are_peer_client_step_bound_and_one_use() {
    let entries = Continuations::default();
    let bob = peer();
    entries
        .insert(
            &bob,
            "app.game",
            Step::Login,
            "secret".into(),
            100,
            permits(1),
        )
        .unwrap();
    let alice = SharePeer {
        user_id: "@alice:example.org".into(),
        ..peer()
    };
    let renewed = SharePeer {
        generation: 8,
        ..peer()
    };
    for (who, client, step) in [
        (&alice, "app.game", Step::Login),
        (&renewed, "app.game", Step::Login),
        (&bob, "app.other", Step::Login),
        (&bob, "app.game", Step::Consent),
    ] {
        assert!(
            entries
                .check(who, client, step, "secret", 101, &permits(1), true)
                .is_err()
        );
    }
    assert!(
        entries
            .check(
                &bob,
                "app.game",
                Step::Login,
                "unknown-owner-verifier",
                101,
                &permits(1),
                true
            )
            .is_err()
    );
    assert!(
        entries
            .check(
                &bob,
                "app.game",
                Step::Login,
                "secret",
                101,
                &permits(2),
                false
            )
            .is_err()
    );
    entries
        .check(
            &bob,
            "app.game",
            Step::Login,
            "secret",
            101,
            &permits(1),
            false,
        )
        .unwrap();
    entries
        .check(
            &bob,
            "app.game",
            Step::Login,
            "secret",
            101,
            &permits(1),
            true,
        )
        .unwrap();
    assert!(
        entries
            .check(
                &bob,
                "app.game",
                Step::Login,
                "secret",
                101,
                &permits(1),
                true
            )
            .is_err()
    );
}

#[test]
fn verifier_expiry_is_exact_and_capacity_fails_closed() {
    let entries = Continuations::default();
    entries
        .insert(
            &peer(),
            "app.game",
            Step::Consent,
            "last".into(),
            100,
            permits(1),
        )
        .unwrap();
    entries
        .check(
            &peer(),
            "app.game",
            Step::Consent,
            "last",
            100 + CONTINUATION_TTL_MS - 1,
            &permits(1),
            false,
        )
        .unwrap();
    assert!(
        entries
            .check(
                &peer(),
                "app.game",
                Step::Consent,
                "last",
                100 + CONTINUATION_TTL_MS,
                &permits(1),
                false
            )
            .is_err()
    );
    for i in 0..MAX_CONTINUATIONS {
        entries
            .insert(
                &peer(),
                "app.game",
                Step::Login,
                i.to_string(),
                100,
                permits(1),
            )
            .unwrap();
    }
    assert_eq!(entries.0.lock().unwrap().len(), MAX_CONTINUATIONS);
    assert!(
        entries
            .insert(
                &peer(),
                "app.game",
                Step::Login,
                "overflow".into(),
                100,
                permits(1)
            )
            .is_err()
    );
    entries
        .insert(
            &peer(),
            "app.game",
            Step::Login,
            "fresh".into(),
            100 + CONTINUATION_TTL_MS,
            permits(1),
        )
        .unwrap();
    assert_eq!(entries.0.lock().unwrap().len(), 1);
}

#[test]
fn hydra_redirects_must_continue_the_expected_client_and_step() {
    let origin = "https://auth.wonton.example:8443";
    let valid = format!("{origin}/oauth2/auth?client_id=app.game&consent_verifier=secret");
    assert_eq!(
        continuation(origin, &valid, "app.game", Step::Consent).unwrap(),
        "secret"
    );
    for target in [
        valid.replace("auth.wonton.example", "attacker.example"),
        valid.replace(":8443", ":8444"),
        valid.replace("https:", "http:"),
        valid.replace("app.game", "app.other"),
        valid.replace("consent_verifier", "login_verifier"),
        format!("{valid}#fragment"),
        valid.replace("https://", "https://user@"),
        valid.replace("/oauth2/auth?", "/oauth2/token?"),
    ] {
        assert!(
            continuation(origin, &target, "app.game", Step::Consent).is_err(),
            "{target}"
        );
    }
}

struct Fixture {
    gateway: Arc<Gateway>,
    sharing: Arc<super::super::Sharing>,
    peer: SharePeer,
    grant: String,
    _harness: Harness,
}

impl Fixture {
    async fn new(hydra: Option<crate::oidc::Hydra>) -> Self {
        let harness = Harness::start(None).await;
        let mut manifest = crate::plugin::PluginPackage::manifest_of(
            crate::gateway::tests::WHOAMI,
            &crate::plugin::languages::Languages::default(),
        )
        .unwrap();
        let webui = manifest.webui.as_mut().unwrap();
        webui.sso = crate::plugin::manifest::Sso::Oidc;
        webui.oidc = Some(crate::plugin::manifest::OidcClient {
            redirect_paths: vec!["/callback".into()],
            post_logout_paths: Vec::new(),
            token_endpoint_auth_method: Default::default(),
            owner_groups: vec!["admin".into()],
        });
        harness.manager.register_for_test(
            manifest,
            crate::plugin::PluginStatus::Running,
            Some(harness.upstream.to_string()),
        );
        let friends = Arc::new(Friends::open(harness.data_dir.join("oidc-friends.json")).unwrap());
        let friend = friends
            .accept("@bob:example.org", Some("Bob"), 1, |_| false)
            .unwrap();
        let shares = Arc::new(
            Shares::with_clock(
                harness.data_dir.join("oidc-shares.json"),
                friends.clone(),
                Arc::new(|| 1),
            )
            .unwrap(),
        );
        let offer = crate::shares::PluginOffer {
            plugin_id: "app.whoami".into(),
            installed: true,
            has_webui: true,
            friends_supported: false,
        };
        let grant = shares
            .create(
                &ShareGrantUpdate {
                    plugin_id: offer.plugin_id.clone(),
                    recipient: ShareRecipient::Friend {
                        user_id: friend.user_id.clone(),
                    },
                    expires_at_ms: None,
                    acknowledge_risk: true,
                },
                &offer,
            )
            .unwrap();
        let identity =
            Arc::new(crate::share_identity::IdentityVerifier::new(Arc::new(|_| None)).unwrap());
        let sharing = Arc::new(super::super::Sharing::new(friends, shares, identity));
        let gateway = Gateway::new(
            crate::config::PublicUrls {
                scheme: "http".into(),
                base_domain: "wonton.localhost".into(),
                port: 80,
            },
            crate::owner::OwnerStore::for_test(&harness.data_dir, &owner()),
            TOKEN.into(),
            harness.members.clone(),
            harness.manager.clone(),
            hydra.map(|h| (h, "http://127.0.0.1:1".into())),
            harness.http.clone(),
            harness.tunnel.clone(),
        )
        .with_sharing(Some(sharing.clone()));
        Self {
            gateway: Arc::new(gateway),
            sharing,
            peer: SharePeer {
                user_id: friend.user_id,
                generation: friend.access_generation,
            },
            grant: grant.id,
            _harness: harness,
        }
    }

    fn request(&self) -> Request {
        let mut req = Request::builder()
            .uri("/_wonton/oidc/login?login_challenge=x")
            .header(
                "cookie",
                format!(
                    "{}={}",
                    crate::gateway::CORE_COOKIE,
                    self.gateway.sessions.create_core(&Person::owner("owner"))
                ),
            )
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(self.peer.clone());
        req
    }
}

#[tokio::test]
async fn share_identity_never_falls_back_to_the_owner_cookie() {
    let fixture = Fixture::new(None).await;
    let mut req = fixture.request();
    assert_eq!(
        person(&fixture.gateway, &req).unwrap().unwrap().role,
        Role::Friend
    );
    req.extensions_mut().remove::<SharePeer>();
    assert_eq!(
        person(&fixture.gateway, &req).unwrap().unwrap().role,
        Role::Owner
    );
    req.extensions_mut().insert(fixture.peer.clone());
    fixture
        .sharing
        .friends
        .remove(&fixture.peer.user_id, 0)
        .unwrap();
    assert!(person(&fixture.gateway, &req).is_err());
    let query = Query::<sso::LoginChallenge>::try_from_uri(req.uri()).unwrap();
    let response = sso::oidc_login(State(fixture.gateway.clone()), query, req).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn every_challenge_and_auth_request_rechecks_current_share_and_installation() {
    let fixture = Fixture::new(None).await;
    let uri = "/oauth2/auth?client_id=app.whoami&response_type=code"
        .parse()
        .unwrap();
    assert_eq!(
        authorize_auth(&fixture.gateway, &fixture.peer, &Method::GET, &uri)
            .unwrap()
            .as_deref(),
        Some("app.whoami")
    );
    let mut req = fixture.request();
    assert_eq!(
        authorize_challenge(&fixture.gateway, &mut req, "app.whoami")
            .unwrap()
            .unwrap()
            .role,
        Role::Friend
    );
    assert!(req.extensions().get::<AccessGuard>().is_some());
    assert!(authorize_challenge(&fixture.gateway, &mut req, "app.uninstalled").is_err());
    fixture.sharing.shares.revoke(&fixture.grant).unwrap();
    assert!(authorize_challenge(&fixture.gateway, &mut req, "app.whoami").is_err());
    assert!(authorize_auth(&fixture.gateway, &fixture.peer, &Method::GET, &uri).is_err());
    let offer = crate::shares::PluginOffer {
        plugin_id: "app.uninstalled".into(),
        installed: true,
        has_webui: true,
        friends_supported: true,
    };
    fixture
        .sharing
        .shares
        .create(
            &ShareGrantUpdate {
                plugin_id: offer.plugin_id.clone(),
                recipient: ShareRecipient::Friend {
                    user_id: fixture.peer.user_id.clone(),
                },
                expires_at_ms: None,
                acknowledge_risk: false,
            },
            &offer,
        )
        .unwrap();
    assert!(authorize_challenge(&fixture.gateway, &mut req, "app.uninstalled").is_err());
    assert_eq!(
        while_active(
            req.extensions().get::<AccessGuard>().cloned(),
            std::future::pending::<()>()
        )
        .await,
        Err(Refusal::ShareRevoked)
    );
}

#[tokio::test]
async fn expired_share_cannot_start_or_accept_an_oidc_challenge() {
    let fixture = Fixture::new(None).await;
    fixture
        .sharing
        .shares
        .update(
            &fixture.grant,
            &ShareGrantUpdate {
                plugin_id: "app.whoami".into(),
                recipient: ShareRecipient::Friend {
                    user_id: fixture.peer.user_id.clone(),
                },
                expires_at_ms: Some(2),
                acknowledge_risk: true,
            },
            &crate::shares::PluginOffer {
                plugin_id: "app.whoami".into(),
                installed: true,
                has_webui: true,
                friends_supported: true,
            },
        )
        .unwrap();
    let uri = "/oauth2/auth?client_id=app.whoami&response_type=code"
        .parse()
        .unwrap();
    assert_eq!(
        authorize_auth(&fixture.gateway, &fixture.peer, &Method::GET, &uri),
        Err(Refusal::ShareExpired)
    );
    assert!(authorize_challenge(&fixture.gateway, &mut fixture.request(), "app.whoami").is_err());
}

#[test]
fn friend_claims_never_include_owner_groups_or_the_owner_email() {
    let friend = crate::owner::Owner {
        username: "bob-plugin".into(),
        display_name: "Bob".into(),
        email: None,
    };
    let claims = crate::oidc::id_token_claims(
        &friend,
        Role::Friend,
        &["profile".into(), "email".into(), "groups".into()],
        &["admin".into(), "owner".into()],
        None,
    );
    assert_eq!(claims["preferred_username"], "bob-plugin");
    assert_eq!(claims["groups"], json!(["friend"]));
    // 好友的邮箱是按固定用户名派生的内部地址，不套主人的邮箱和组。
    assert_eq!(claims["email"], "bob-plugin@wonton.internal");
    assert_eq!(claims["email_verified"], json!(true));
}

async fn hydra_for_friend(
    username: String,
    accepted: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
) -> crate::oidc::Hydra {
    use axum::{Json, Router, routing::get};
    use std::collections::HashMap;
    let (listener, addr) = crate::gateway::fakes::bind().await;
    let login_accept = accepted.clone();
    let app = Router::new()
        .route("/admin/oauth2/auth/requests/login", get(|Query(q): Query<HashMap<String, String>>| async move {
            Json(json!({"skip": false, "subject": "", "client": {"client_id": q["login_challenge"]}}))
        }))
        .route("/admin/oauth2/auth/requests/login/accept", axum::routing::put(move |Query(q): Query<HashMap<String, String>>, Json(body): Json<serde_json::Value>| {
            let accepted = login_accept.clone();
            async move {
                accepted.lock().unwrap().push(body);
                Json(json!({"redirect_to": format!("http://auth.wonton.localhost/oauth2/auth?client_id={}&login_verifier=login-secret", q["login_challenge"])}))
            }
        }))
        .route("/admin/oauth2/auth/requests/consent", get(move |Query(q): Query<HashMap<String, String>>| {
            let username = username.clone();
            async move { Json(json!({"subject": username, "client": {"client_id": q["consent_challenge"]}, "requested_scope": ["openid", "profile", "groups"]})) }
        }))
        .route("/admin/oauth2/auth/requests/consent/accept", axum::routing::put(move |Query(q): Query<HashMap<String, String>>, Json(body): Json<serde_json::Value>| {
            let accepted = accepted.clone();
            async move {
                accepted.lock().unwrap().push(body);
                Json(json!({"redirect_to": format!("http://auth.wonton.localhost/oauth2/auth?client_id={}&consent_verifier=consent-secret", q["consent_challenge"])}))
            }
        }));
    crate::gateway::fakes::serve(listener, app).await;
    crate::oidc::Hydra::new(
        crate::http::HttpClient::new(),
        &crate::config::OidcProviderConfig {
            admin_url: format!("http://{addr}"),
            public_url: format!("http://{addr}"),
        },
    )
}

async fn callback(fixture: &Fixture, client: &str, consent: bool) -> axum::response::Response {
    let mut req = fixture.request();
    *req.uri_mut() = format!(
        "/_wonton/oidc/{}?{}_challenge={client}",
        if consent { "consent" } else { "login" },
        if consent { "consent" } else { "login" }
    )
    .parse()
    .unwrap();
    if consent {
        let query = Query::<sso::ConsentChallenge>::try_from_uri(req.uri()).unwrap();
        sso::oidc_consent(State(fixture.gateway.clone()), query, req).await
    } else {
        let query = Query::<sso::LoginChallenge>::try_from_uri(req.uri()).unwrap();
        sso::oidc_login(State(fixture.gateway.clone()), query, req).await
    }
}

#[tokio::test]
async fn login_and_consent_accept_only_the_live_shared_client_as_friend() {
    let mut fixture = Fixture::new(None).await;
    let accepted = Arc::new(std::sync::Mutex::new(Vec::new()));
    let username = fixture
        .sharing
        .friends
        .get(&fixture.peer.user_id)
        .unwrap()
        .username;
    let hydra = hydra_for_friend(username.clone(), accepted.clone()).await;
    Arc::get_mut(&mut fixture.gateway).unwrap().hydra = Some(hydra);
    for consent in [false, true] {
        assert_eq!(
            callback(&fixture, "app.other", consent).await.status(),
            StatusCode::FORBIDDEN
        );
    }
    assert!(accepted.lock().unwrap().is_empty());
    assert_eq!(
        callback(&fixture, "app.whoami", false).await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        callback(&fixture, "app.whoami", true).await.status(),
        StatusCode::SEE_OTHER
    );
    {
        let accepted = accepted.lock().unwrap();
        assert_eq!(accepted[0]["subject"], username);
        assert_eq!(
            accepted[1]["session"]["id_token"]["preferred_username"],
            username
        );
        assert_eq!(
            accepted[1]["session"]["id_token"]["groups"],
            json!(["friend"])
        );
    }
    let uri = "/oauth2/auth?client_id=app.whoami&consent_verifier=consent-secret"
        .parse()
        .unwrap();
    assert!(authorize_auth(&fixture.gateway, &fixture.peer, &Method::GET, &uri).is_ok());
    let other = SharePeer {
        user_id: "@alice:example.org".into(),
        generation: fixture.peer.generation,
    };
    assert!(authorize_auth(&fixture.gateway, &other, &Method::GET, &uri).is_err());
    consume_auth(&fixture.gateway, &fixture.peer, &Method::GET, &uri).unwrap();
    assert!(consume_auth(&fixture.gateway, &fixture.peer, &Method::GET, &uri).is_err());
    fixture.sharing.shares.revoke(&fixture.grant).unwrap();
    for consent in [false, true] {
        assert_eq!(
            callback(&fixture, "app.whoami", consent).await.status(),
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(accepted.lock().unwrap().len(), 2);
}
