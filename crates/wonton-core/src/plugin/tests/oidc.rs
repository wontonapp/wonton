use std::sync::{Arc, Mutex};

use axum::routing::{get, post, put};
use axum::{Json, Router};
use hyper::{Method, StatusCode};
use serde_json::{Value, json};

use super::super::fake_docker::FakeDocker;
use super::super::{PluginPackage, PluginStatus};
use super::egress::serve;
use super::manager::manager_with_oidc;
use super::package;
use crate::config::OidcProviderConfig;
use crate::http::HttpClient;
use crate::oidc::Hydra;

#[derive(Default)]
struct Client {
    registered: Option<Value>,
    requests: Vec<(Method, Value)>,
}

type Provider = Arc<Mutex<Client>>;

async fn fake_hydra(client: Provider) -> Hydra {
    // 和 Hydra v26.2.0 实测的一样：授权、登出端点给浏览器地址，签发方和后台端点给内部地址。
    let public = |path: &str| format!("http://auth.wonton.localhost:8080{path}");
    let internal = |path: &str| format!("http://auth.wonton.internal:8080{path}");
    let discovery = json!({
        "issuer": internal(""),
        "authorization_endpoint": public("/oauth2/auth"),
        "token_endpoint": internal("/oauth2/token"),
        "userinfo_endpoint": internal("/userinfo"),
        "jwks_uri": internal("/.well-known/jwks.json"),
        "revocation_endpoint": internal("/oauth2/revoke"),
        "end_session_endpoint": public("/oauth2/sessions/logout"),
    });
    let update = client.clone();
    let app = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(move || async move { Json(discovery) }),
        )
        .route(
            "/admin/clients/{id}",
            put(move |Json(body): Json<Value>| async move {
                let mut client = update.lock().unwrap();
                client.requests.push((Method::PUT, body.clone()));
                if client.registered.is_none() {
                    return StatusCode::NOT_FOUND;
                }
                client.registered = Some(body);
                StatusCode::OK
            }),
        )
        .route(
            "/admin/clients",
            post(move |Json(body): Json<Value>| async move {
                let mut client = client.lock().unwrap();
                client.requests.push((Method::POST, body.clone()));
                client.registered = Some(body.clone());
                (StatusCode::CREATED, Json(body))
            }),
        );
    let addr = serve(app).await;
    Hydra::new(
        HttpClient::new(),
        &OidcProviderConfig {
            admin_url: format!("http://{addr}"),
            public_url: format!("http://{addr}"),
        },
    )
}

fn app(port: u16, logout: &str) -> String {
    package(
        "  web:\n    image: nginx",
        &format!(
            "  permissions: []\n  webui:\n    service: web\n    port: {port}\n    \
             path: /oidc/login?provider=wonton\n    sso: oidc\n    oidc:\n      \
             redirect_paths: [/oidc/callback]\n{logout}"
        ),
    )
}

#[tokio::test]
async fn oidc_logout_defaults_are_registered_on_create_and_restored_without_reinstall() {
    let web = serve(Router::new().route("/", get(|| async { "ok" }))).await;
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let client = Provider::default();
    let hydra = fake_hydra(client.clone()).await;
    let manager = manager_with_oidc(engine.clone(), &dir, hydra.clone());
    manager
        .install(&app(web.port(), ""), &[], super::langs())
        .await
        .unwrap();
    let base = "http://app-test.wonton.localhost:8080";
    let expected = json!([
        base,
        format!("{base}/"),
        format!("{base}/oidc/login?provider=wonton")
    ]);
    let original = {
        let mut client = client.lock().unwrap();
        assert_eq!(client.requests[0].0, Method::PUT);
        assert_eq!(client.requests[1].0, Method::POST);
        let body = client.registered.as_mut().unwrap();
        assert_eq!(body["post_logout_redirect_uris"], expected);
        assert_eq!(
            body["redirect_uris"],
            json!([format!("{base}/oidc/callback")])
        );
        body.as_object_mut()
            .unwrap()
            .remove("post_logout_redirect_uris");
        body.clone()
    };
    drop(manager);
    let restarted = manager_with_oidc(engine, &dir, hydra);
    restarted.restore().await.unwrap();
    assert_eq!(restarted.list()[0].status, PluginStatus::Running);
    let client = client.lock().unwrap();
    let (method, body) = client.requests.last().unwrap();
    assert_eq!(*method, Method::PUT);
    assert_eq!(body["post_logout_redirect_uris"], expected);
    assert_eq!(body["client_secret"], original["client_secret"]);
}

#[tokio::test]
async fn oidc_logout_declared_paths_extend_defaults_without_duplicates() {
    let web = serve(Router::new().route("/", get(|| async { "ok" }))).await;
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let client = Provider::default();
    let hydra = fake_hydra(client.clone()).await;
    let manager = manager_with_oidc(engine, &dir, hydra);
    let paths = "      post_logout_paths: [/, /auth, /auth, '/auth?logged_out=1']\n";
    manager
        .install(&app(web.port(), paths), &[], super::langs())
        .await
        .unwrap();
    let client = client.lock().unwrap();
    let body = client.registered.as_ref().unwrap();
    let base = "http://app-test.wonton.localhost:8080";
    assert_eq!(
        body["post_logout_redirect_uris"],
        json!([
            base,
            format!("{base}/"),
            format!("{base}/auth"),
            format!("{base}/auth?logged_out=1"),
            format!("{base}/oidc/login?provider=wonton")
        ])
    );
}

#[test]
fn oidc_logout_paths_require_a_leading_slash_like_login_redirects() {
    for path in ["", "auth", "https://example.com/auth"] {
        let compose = app(80, &format!("      post_logout_paths: [{path:?}]\n"));
        assert!(
            PluginPackage::parse(&compose, &super::langs()).is_err(),
            "{path:?}"
        );
    }
    for paths in ["[]", "[/auth]", "['/auth?state=logout']"] {
        PluginPackage::parse(
            &app(80, &format!("      post_logout_paths: {paths}\n")),
            &super::langs(),
        )
        .unwrap();
    }
}

#[tokio::test]
async fn injected_endpoints_follow_who_reaches_them() {
    let web = serve(Router::new().route("/", get(|| async { "ok" }))).await;
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with_oidc(engine, &dir, fake_hydra(Provider::default()).await);
    let compose = app(web.port(), "").replace(
        "    image: nginx",
        "    image: nginx\n    environment:\n      \
         WONTON_OIDC_ISSUER: ${WONTON_OIDC_ISSUER}\n      \
         WONTON_OIDC_AUTH_URL: ${WONTON_OIDC_AUTH_URL}\n      \
         WONTON_OIDC_TOKEN_URL: ${WONTON_OIDC_TOKEN_URL}\n      \
         WONTON_OIDC_USERINFO_URL: ${WONTON_OIDC_USERINFO_URL}",
    );
    manager
        .install(&compose, &[], super::langs())
        .await
        .unwrap();
    let env = super::env_of_container(
        &docker,
        &super::super::names::container_name("app-test", "web"),
    );
    let public = "http://auth.wonton.localhost:8080";
    let internal = "http://auth.wonton.internal:8080";
    assert_eq!(env["WONTON_OIDC_ISSUER"], internal);
    assert_eq!(env["WONTON_OIDC_AUTH_URL"], format!("{public}/oauth2/auth"));
    assert_eq!(
        env["WONTON_OIDC_TOKEN_URL"],
        format!("{internal}/oauth2/token")
    );
    assert_eq!(
        env["WONTON_OIDC_USERINFO_URL"],
        format!("{internal}/userinfo")
    );
}
