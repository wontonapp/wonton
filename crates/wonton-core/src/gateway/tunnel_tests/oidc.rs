//! 设备身份在 OIDC 登录和同意页直接算主人，不需要核心会话。

use super::*;

use std::sync::Mutex;

use hyper::StatusCode;
use iroh::SecretKey;

use super::super::fakes::fake_hydra;

#[tokio::test]
async fn device_identity_signs_in_to_oidc_without_a_core_session() {
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let hydra = fake_hydra(accepted.clone()).await;
    let home = Home::start(Some(hydra)).await;
    let device = SecretKey::generate();
    home.register(&device).await;
    let phone = Phone::start(device, &home).await;

    let (status, headers, _) = phone
        .get(CORE, "/_wonton/oidc/login?login_challenge=abc")
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        header(&headers, "location"),
        Some("http://auth.wonton.localhost/after-login")
    );
    let (status, _, _) = phone
        .get(CORE, "/_wonton/oidc/consent?consent_challenge=def")
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let accepted = accepted.lock().unwrap();
    assert_eq!(accepted[0]["subject"], "owner");
    assert_eq!(
        accepted[1]["session"]["id_token"]["preferred_username"],
        "owner"
    );
}
