//! 网页的 OIDC 身份取自当前这个人：家人拿到自己的用户名，主人照旧带能力（ADR-0045）。

use std::sync::{Arc, Mutex};

use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};

use crate::gateway::CORE_COOKIE;
use crate::gateway::fakes::fake_hydra_for;
use crate::gateway::family_tests::send_as;
use crate::gateway::tests::{CORE, Harness, TOKEN};

use super::invite_record;

/// 网页的身份给当前这个人：家人的 OIDC 声明是她的用户名和通用的 `user` 组，主人的照旧加能力。
#[tokio::test]
async fn oidc_claims_come_from_the_current_person() {
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let hydra = fake_hydra_for(accepted.clone(), "mia").await;
    let h = Harness::start(Some(hydra)).await;
    let hers = SecretKey::generate().public();
    invite_record(&h, false).await;
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();

    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        CORE,
        "/_wonton/oidc/consent?consent_challenge=def",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.text());
    let claims = claims_of(&accepted, 0);
    assert_eq!(claims["preferred_username"], "mia");
    assert_eq!(claims["groups"], json!(["user"]));
    assert_eq!(claims["email"], "mia@wonton.internal");
    assert_eq!(claims["email_verified"], true);

    // 主人的声明照旧（主人令牌登录的核心会话）。假 Hydra 对这个 challenge 返回主人的 subject。
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    let reply = send_as(
        &h.app,
        None,
        Method::GET,
        CORE,
        "/_wonton/oidc/consent?consent_challenge=ghi",
        &[("cookie", core.as_str())],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.text());
    let claims = claims_of(&accepted, 1);
    assert_eq!(claims["preferred_username"], "owner");
    assert_eq!(claims["groups"], json!(["user", "plugin_admin"]));
    assert_eq!(claims["email"], "owner@wonton.internal");
    assert_eq!(claims["email_verified"], true);
}

fn claims_of(accepted: &Mutex<Vec<Value>>, nth: usize) -> Value {
    accepted.lock().unwrap()[nth]["session"]["id_token"].clone()
}
