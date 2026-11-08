//! 家里连着、那一次登录请求却失败（满载时网关回 502、要令牌超时、Tuwunel 正在重启）：不用回前台，自己按退避重登，期间显示“正在重连”。

use wonton_client::{ChatAccount, ChatFailure};

use super::support::{Home, OWNER, Phone};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_sign_in_that_fails_while_the_tunnel_stays_up_retries_by_itself() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    home.fail_login_tokens(2);
    phone.connect(&home);

    let reason = phone
        .wait_chat("登录失败后正在重连", |view| match &view.account {
            ChatAccount::Reconnecting { reason, .. } => Some(*reason),
            _ => None,
        })
        .await;
    assert_eq!(reason, ChatFailure::TransportUnavailable);

    let (user_id, _) = phone.signed_in().await;
    assert_eq!(user_id, OWNER);
    assert_eq!(home.login_tokens(), 1);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_matrix_login_refused_while_tuwunel_restarts_retries_by_itself() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // SDK 对 5xx 自己退避重试，登录前取 `/versions` 只试 5 次（约 7 秒）就把 503 交上来：Tuwunel 得一直不在。
    home.set_matrix_unavailable(true);
    phone.connect(&home);

    let reason = phone
        .wait_chat("Matrix 登录失败后正在重连", |view| {
            match &view.account {
                ChatAccount::Reconnecting { reason, .. } => Some(*reason),
                _ => None,
            }
        })
        .await;
    assert_eq!(reason, ChatFailure::NotReady);

    home.set_matrix_unavailable(false);
    let (user_id, _) = phone.signed_in().await;
    assert_eq!(user_id, OWNER);
    assert!(
        home.login_tokens() >= 2,
        "应该重登过：{}",
        home.login_tokens()
    );
}
