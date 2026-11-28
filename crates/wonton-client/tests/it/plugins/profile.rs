//! 主人资料和头像：保存把资料送上去并留住被拒的原因，头像字节先过检查。

use axum::http::Method;
use wonton_client::{CoreError, OwnerProfileUpdate};
use wonton_protocol::{AvatarMediaProblem, AvatarUpload, MyAvatarProblem};

use super::rig::{Canned, Rig, core_error};

const PROFILE: &str = wonton_protocol::ME_PATH;

fn profile(name: Option<&str>, email: Option<&str>) -> Canned {
    let body = serde_json::json!({
        "user_id": "@owner:wonton.localhost",
        "username": "owner",
        "is_owner": true,
        "display_name": name,
        "email": email,
    });
    Canned(200, body.to_string())
}

#[tokio::test]
async fn saving_the_profile_sends_it_and_keeps_the_reason_it_was_refused() {
    let rig = Rig::new().await;
    rig.connected().await;

    let update = OwnerProfileUpdate {
        display_name: "云吞".to_owned(),
        email: Some("bad".to_owned()),
    };
    let refused = core_error(400, "invalid_input", "不像邮箱");
    rig.core.answer(Method::PUT, PROFILE, refused);
    let error = rig.app.save_me(update.clone()).await.unwrap_err();
    assert_eq!(
        error,
        CoreError::InvalidInput {
            reason: "不像邮箱".to_owned()
        }
    );

    let saved = profile(Some("云吞"), Some("me@example.com"));
    rig.core.answer(Method::PUT, PROFILE, saved);
    let good = OwnerProfileUpdate {
        email: Some("me@example.com".to_owned()),
        ..update
    };
    let profile = rig.app.save_me(good).await.unwrap();
    assert_eq!(profile.email.as_deref(), Some("me@example.com"));
    let sent = rig.core.seen(&Method::PUT, PROFILE);
    assert!(sent[1].body.contains("me@example.com"), "{}", sent[1].body);
    assert!(!sent[1].from_page);
}

#[tokio::test]
async fn avatar_bytes_are_checked_before_the_upload() {
    let rig = Rig::new().await;
    let upload = AvatarUpload {
        mime_type: "image/jpeg".to_owned(),
        data: vec![1, 2, 3],
        poster: None,
    };
    assert_eq!(
        rig.app.set_my_avatar(upload).await.unwrap_err(),
        MyAvatarProblem::MediaRejected {
            problem: AvatarMediaProblem::Format
        }
    );
    // 字节先过检查，这一步还没连上 Matrix，清头像说聊天还没登录。
    assert!(matches!(
        rig.app.clear_my_avatar().await.unwrap_err(),
        MyAvatarProblem::NotLoggedIn { .. }
    ));
    let png = AvatarUpload {
        mime_type: "image/png".to_owned(),
        data: include_bytes!("../../../../wonton-matrix/tests/tuwunel/fixtures/2x2.png").to_vec(),
        poster: None,
    };
    assert!(matches!(
        rig.app.set_my_avatar(png).await.unwrap_err(),
        MyAvatarProblem::NotLoggedIn { .. }
    ));
}
