//! OIDC 登录和授权确认：走真实 TCP 连接，Hydra 管理接口用替身。声明的内容（scope、声明）在这里验证。

use std::sync::{Arc, Mutex};

use hyper::StatusCode;
use serde_json::json;

use super::CORE_COOKIE;
use super::tests::{CORE, Harness, TOKEN, WHOAMI, fake_hydra};
use crate::plugin::languages::Languages;
use crate::plugin::{PluginPackage, PluginStatus};

#[tokio::test]
async fn oidc_login_and_consent_use_the_core_session() {
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let hydra = fake_hydra(accepted.clone()).await;
    let h = Harness::start(Some(hydra)).await;

    let anonymous = h
        .get(CORE, "/_wonton/oidc/login?login_challenge=abc", &[])
        .await;
    assert_eq!(anonymous.status, StatusCode::SEE_OTHER);
    assert!(anonymous.location().starts_with("/_wonton/login?next="));
    assert!(
        accepted.lock().unwrap().is_empty(),
        "未登录时不能接受 OIDC 登录"
    );

    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    let login = h
        .get(
            CORE,
            "/_wonton/oidc/login?login_challenge=abc",
            &[("cookie", &core)],
        )
        .await;
    assert_eq!(login.location(), "http://auth.wonton.localhost/after-login");
    let consent = h
        .get(
            CORE,
            "/_wonton/oidc/consent?consent_challenge=def",
            &[("cookie", &core)],
        )
        .await;
    assert_eq!(
        consent.location(),
        "http://auth.wonton.localhost/after-consent"
    );

    let accepted = accepted.lock().unwrap();
    assert_eq!(accepted[0]["subject"], "owner");
    assert_eq!(
        accepted[1]["grant_scope"],
        json!(["openid", "profile", "email", "groups"])
    );
    let claims = &accepted[1]["session"]["id_token"];
    assert_eq!(claims["email"], "owner@wonton.localhost");
    assert_eq!(claims["preferred_username"], "owner");
    assert_eq!(claims["groups"], json!(["owner"]));
}

/// 声明了 `oidc.owner_groups` 的客户端拿到 `["owner", …]`，没声明的客户端只有 `["owner"]`。
/// Hydra 把 `session.id_token` 的声明同时用作 ID 令牌和 userinfo 的内容。
#[tokio::test]
async fn consent_gives_owner_groups_to_the_declaring_client_only() {
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let hydra = fake_hydra(accepted.clone()).await;
    let h = Harness::start(Some(hydra)).await;

    let books = PluginPackage::manifest_of(
        &WHOAMI.replace("app.whoami", "app.books").replace(
            "  webui:\n    service: web\n    port: 8000",
            "  webui:\n    service: web\n    port: 8000\n    sso: oidc\n    oidc:\n      \
                 redirect_paths: [/cb]\n      owner_groups: [admin, app.admin]",
        ),
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    assert_eq!(books.id.as_str(), "app.books");
    h.manager
        .register_for_test(books, PluginStatus::Running, None);

    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    let books_consent = h
        .get(
            CORE,
            "/_wonton/oidc/consent?consent_challenge=app.books",
            &[("cookie", &core)],
        )
        .await;
    assert_eq!(
        books_consent.location(),
        "http://auth.wonton.localhost/after-consent"
    );
    // 没声明的客户端（Harness 里登记的 whoami，客户端 ID 也是 `app.whoami`）。
    let plain_consent = h
        .get(
            CORE,
            "/_wonton/oidc/consent?consent_challenge=app.whoami",
            &[("cookie", &core)],
        )
        .await;
    assert_eq!(
        plain_consent.location(),
        "http://auth.wonton.localhost/after-consent"
    );

    let accepted = accepted.lock().unwrap();
    assert_eq!(
        accepted[0]["session"]["id_token"]["groups"],
        json!(["owner", "admin", "app.admin"])
    );
    assert_eq!(
        accepted[1]["session"]["id_token"]["groups"],
        json!(["owner"])
    );
}
