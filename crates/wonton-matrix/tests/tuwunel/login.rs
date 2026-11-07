//! 核心签发 JWT 登录（ADR-0009）：令牌的有效期与校验、重放，访问令牌过期后的刷新，以及吊销。

use std::time::Duration;

use futures_util::StreamExt;
use wonton_matrix::{Error, MatrixSession, Result, SessionEvent, SyncStatus};

use crate::support::{Claims, Issuer, TestServer, diagnose_live_session, eventually, now, unique};

/// 登录被拒绝，且错误码是 `errcode`。
#[track_caller]
fn assert_rejected(result: Result<MatrixSession>, errcode: &str) {
    match result {
        Err(Error::Unauthorized {
            errcode: actual, ..
        }) if actual == errcode => {}
        Err(other) => panic!("应被拒绝（{errcode}），实际：{other:?}"),
        Ok(session) => panic!("应被拒绝（{errcode}），实际登录成功：{}", session.user_id()),
    }
}

/// 已登录的会话失效：请求被拒绝，并收到“令牌失效、不是软登出”的会话事件，客户端据此提示重新配对。
///
/// 错误码不固定：令牌失效（`M_UNKNOWN_TOKEN`）后 SDK 会先用刷新令牌重试，Tuwunel 对已删除的刷新令牌
/// 返回 `M_FORBIDDEN`（规范要求 `M_UNKNOWN_TOKEN`），最终报出的是刷新失败。
async fn assert_signed_out(
    error: &Error,
    events: std::pin::Pin<&mut impl futures_util::Stream<Item = SessionEvent>>,
) {
    assert!(
        matches!(error, Error::Unauthorized { .. }),
        "应是权限错误，实际：{error:?}"
    );
    let event = eventually("收到令牌失效事件", events, |event| Some(*event)).await;
    assert_eq!(event, SessionEvent::TokenRejected { soft_logout: false });
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn login_token_must_be_valid_and_from_the_core() {
    let server = TestServer::container(&[]).await;
    let user = unique("dana");
    let token = |claims: Claims| server.issuer.sign(&claims);

    // 过期判断有 60 秒的时钟偏差容忍：过期 30 秒仍可登录，过期 90 秒被拒绝。
    let expired = |seconds_ago| Claims {
        exp: Some(now() - seconds_ago),
        ..Claims::login(&user)
    };
    assert!(server.try_login(&token(expired(30))).await.is_ok());
    assert_rejected(server.try_login(&token(expired(90))).await, "M_FORBIDDEN");

    // 必须带 exp，签发方和接收方必须是核心和 Tuwunel。
    assert_rejected(
        server
            .try_login(&token(Claims {
                exp: None,
                ..Claims::login(&user)
            }))
            .await,
        "M_FORBIDDEN",
    );
    let wrong_issuer = Claims {
        iss: Some("someone-else".to_owned()),
        ..Claims::login(&user)
    };
    assert_rejected(server.try_login(&token(wrong_issuer)).await, "M_FORBIDDEN");
    let wrong_audience = Claims {
        aud: Some("another-service".to_owned()),
        ..Claims::login(&user)
    };
    assert_rejected(
        server.try_login(&token(wrong_audience)).await,
        "M_FORBIDDEN",
    );

    // 别的密钥签的令牌被拒绝。
    let forged = Issuer::generate().login_token(&user);
    assert_rejected(server.try_login(&forged).await, "M_FORBIDDEN");
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn login_token_is_not_single_use() {
    let server = TestServer::container(&[]).await;
    let jwt = server.issuer.login_token(&unique("frank"));

    // Tuwunel 不检查 jti：有效期内同一个令牌可以重复登录，每次得到一台新设备。
    let first = server.try_login(&jwt).await.expect("第一次登录");
    let second = server.try_login(&jwt).await.expect("重放同一个令牌");
    assert_eq!(first.user_id(), second.user_id());
    assert_ne!(first.device_id(), second.device_id());
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn expired_access_token_is_refreshed() {
    let server = TestServer::container(&[("TUWUNEL_ACCESS_TOKEN_TTL", "2")]).await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let mut events = std::pin::pin!(alice.session_events());

    tokio::time::sleep(Duration::from_secs(3)).await;
    alice.start_sync().await.expect("访问令牌过期后仍能同步");
    let event = tokio::time::timeout(Duration::from_secs(10), events.next()).await;
    assert_eq!(
        event.expect("应发出会话事件"),
        Some(SessionEvent::TokensRefreshed)
    );
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn revoked_device_is_signed_out_and_can_pair_again() {
    let server = TestServer::container(&[]).await;
    let admin = server.admin().await;
    let user = unique("erin");
    let phone = server.login(&user, "手机").await;
    let tablet = server.login(&user, "平板").await;
    phone.start_sync().await.expect("手机开始同步");
    tablet.start_sync().await.expect("平板开始同步");
    let mut phone_events = std::pin::pin!(phone.session_events());

    admin
        .delete_device(phone.user_id(), phone.device_id())
        .await;

    // 手机的同步失败：令牌已失效且不是软登出，只能重新配对。
    let error = eventually(
        "手机的同步失败",
        phone.sync_status(),
        |status| match status {
            SyncStatus::Failed(error) => Some(error.clone()),
            _ => None,
        },
    )
    .await;
    assert_signed_out(&error, phone_events.as_mut()).await;

    // 同一账号的平板不受影响：仍在同步，也能继续发起请求。
    let status = std::pin::pin!(tablet.sync_status()).next().await;
    assert_eq!(status, Some(SyncStatus::Running));
    let friend = server.login(&unique("frank"), "Frank 的手机").await;
    assert!(
        tablet
            .create_direct_chat(friend.user_id(), false)
            .await
            .is_ok(),
        "平板应仍可使用"
    );

    // 重新配对：核心签发新令牌，登录得到一台新设备。
    let new_phone = server.login(&user, "手机").await;
    let mut devices = admin.device_ids(phone.user_id()).await;
    devices.sort();
    let mut expected = vec![
        tablet.device_id().to_string(),
        new_phone.device_id().to_string(),
    ];
    expected.sort();
    assert_eq!(devices, expected);

    // 用户在新手机上主动退出登录：服务器删除这台设备。
    new_phone.logout().await.expect("退出登录");
    assert_eq!(
        admin.device_ids(phone.user_id()).await,
        vec![tablet.device_id().to_string()]
    );
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn deactivated_bot_cannot_log_in_again() {
    let server = TestServer::container(&[]).await;
    let admin = server.admin().await;
    let localpart = unique("hermes");
    let bot = server.login(&localpart, "Hermes").await;
    let owner = server.login(&unique("owner"), "主人的手机").await;
    let mut events = std::pin::pin!(bot.session_events());

    // 停用前取令牌：建房间时 SDK 可能已经刷新过，那时会话里的就不是被停用的那一对了。
    let (access_token, refresh_token) = bot.tokens().expect("session should report its tokens");
    admin.deactivate(bot.user_id()).await;
    let deactivated_at = std::time::Instant::now();

    let error = match bot.create_direct_chat(owner.user_id(), false).await {
        Err(error) => error,
        // 停用后的会话不应该还能建房间。真发生了，先用旧令牌裸打一次 whoami、一次 refresh，
        // 把服务器当时的回答连同相对停用返回的毫秒数写进 panic，再失败。
        Ok(_) => {
            diagnose_live_session(
                &server.url,
                &access_token,
                refresh_token.as_deref(),
                deactivated_at,
            )
            .await
        }
    };
    assert_signed_out(&error, events.as_mut()).await;
    let token = server.issuer.login_token(&localpart);
    assert_rejected(server.try_login(&token).await, "M_USER_DEACTIVATED");
}
