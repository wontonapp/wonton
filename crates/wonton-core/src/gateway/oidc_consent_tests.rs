//! OIDC 登录和授权确认：走真实 TCP 连接，Hydra 管理接口用替身。声明的内容（scope、声明）在这里验证。

use std::sync::{Arc, Mutex};

use hyper::StatusCode;
use serde_json::json;

use super::CORE_COOKIE;
use super::fakes::{fake_hydra, fake_hydra_for, fake_hydra_scopes};
use super::family_tests::send_as;
use super::tests::{CORE, Harness, Reply, TOKEN, WHOAMI};
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
    assert_eq!(claims["email"], "owner@wonton.internal");
    assert_eq!(claims["email_verified"], true);
    assert_eq!(claims["preferred_username"], "owner");
    assert_eq!(claims["groups"], json!(["user", "plugin_admin"]));
}

/// 声明了 `oidc.admin_groups` 的客户端在 `plugin_admin` 之外拿到那些组，没声明的只有能力组。
/// Hydra 把 `session.id_token` 的声明同时用作 ID 令牌和 userinfo 的内容。
#[tokio::test]
async fn consent_gives_admin_groups_to_the_declaring_client_only() {
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let hydra = fake_hydra(accepted.clone()).await;
    let h = Harness::start(Some(hydra)).await;

    let books = PluginPackage::manifest_of(
        &WHOAMI.replace("app.whoami", "app.books").replace(
            "  webui:\n    service: web\n    port: 8000",
            "  webui:\n    service: web\n    port: 8000\n    sso: oidc\n    oidc:\n      \
                 redirect_paths: [/cb]\n      admin_groups: [admin, app.admin]",
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
        json!(["user", "plugin_admin", "admin", "app.admin"])
    );
    assert_eq!(
        accepted[1]["session"]["id_token"]["groups"],
        json!(["user", "plugin_admin"])
    );
}

/// 给一个人配一台设备、记下他的语言；设备归属决定 OIDC 认成谁。
fn person(h: &Harness, username: &str, language: &str) -> iroh::EndpointId {
    if username != "owner" {
        h.members
            .invite(crate::members::MemberRecord {
                username: username.into(),
                user_id: format!("@{username}:{}", "test"),
                display_name: username.into(),
                email: None,
                status: wonton_protocol::MemberStatus::Joined,
                role_id: "friend".to_owned(),
                wants_private_bot: false,
                private_bot: None,
                pairing_code_hash: None,
                pairing_code_expires_at_ms: None,
            })
            .unwrap();
    }
    let id = iroh::SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(
            id,
            None,
            (username != "owner").then(|| username.to_owned()),
            |_| true,
        )
        .unwrap();
    h.manager.note_person_languages(username, Some(language));
    id
}

async fn consent_as(
    h: &Harness,
    device: Option<iroh::EndpointId>,
    challenge: &str,
    cookie: Option<&str>,
) -> Reply {
    let path = format!("/_wonton/oidc/consent?consent_challenge={challenge}");
    match cookie {
        Some(cookie) => h.get(CORE, &path, &[("cookie", cookie)]).await,
        None => send_as(&h.app, device, hyper::Method::GET, CORE, &path, &[], "").await,
    }
}

/// `locale` 是登录者本人的（主人和家人各拿各的），B 47 原样；换过语言后下一次签发就是新值。
/// ID 令牌和 userinfo 是同一份 `session.id_token`，值天然一致。
#[tokio::test]
async fn each_person_gets_their_own_locale() {
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let hydra = fake_hydra_for(accepted.clone(), "alice").await;
    let h = Harness::start(Some(hydra)).await;
    person(&h, "owner", "zh-Hant-TW, en;q=0.8");
    let alice = person(&h, "alice", "ja-JP");

    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    assert_eq!(
        h.manager.person_locale("owner").as_deref(),
        Some("zh-Hant-TW"),
        "先有记录"
    );
    assert_eq!(
        consent_as(&h, None, "abc", Some(&core)).await.status,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        consent_as(&h, Some(alice), "def", None).await.status,
        StatusCode::SEE_OTHER
    );

    // 主人换了语言：下一次签发是新值。
    h.manager.note_person_languages("owner", Some("en-GB"));
    assert_eq!(
        consent_as(&h, None, "abc", Some(&core)).await.status,
        StatusCode::SEE_OTHER
    );

    // 每成功一次授权确认记一条。
    let accepted = accepted.lock().unwrap();
    let locale = |i: usize| accepted[i]["session"]["id_token"]["locale"].clone();
    assert_eq!(
        accepted[0]["session"]["id_token"]["preferred_username"],
        "owner"
    );
    assert_eq!(locale(0), "zh-Hant-TW");
    assert_eq!(
        accepted[1]["session"]["id_token"]["preferred_username"],
        "alice"
    );
    assert_eq!(locale(1), "ja-JP");
    assert_eq!(locale(2), "en-GB", "换过语言后是新值");
}

/// 没授予 `profile` 就不发 `locale`；这个人还没有记录时也不发。
#[tokio::test]
async fn locale_needs_the_profile_scope_and_a_record() {
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let hydra = fake_hydra_scopes(accepted.clone(), "owner", &["openid"]).await;
    let h = Harness::start(Some(hydra)).await;
    person(&h, "owner", "ja");
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    assert_eq!(
        consent_as(&h, None, "abc", Some(&core)).await.status,
        StatusCode::SEE_OTHER
    );
    {
        let accepted = accepted.lock().unwrap();
        let claims = &accepted[0]["session"]["id_token"];
        assert!(claims.get("locale").is_none(), "{claims}");
    }

    // 有 `profile` 但这个人没有记录。
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let hydra = super::fakes::fake_hydra(accepted.clone()).await;
    let h = Harness::start(Some(hydra)).await;
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    assert_eq!(
        consent_as(&h, None, "abc", Some(&core)).await.status,
        StatusCode::SEE_OTHER
    );
    let accepted = accepted.lock().unwrap();
    let claims = &accepted[0]["session"]["id_token"];
    assert!(claims.get("locale").is_none(), "{claims}");
}
