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

/// 隧道断了又回来（代理换了地址）：旧代理上的同步只会报“连不上”，自己重启没用，得拿新代理重新登录一次。
/// 消息照常收到，设备不变（是恢复不是新登录），也不来回重登。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_sync_that_only_fails_because_the_proxy_is_gone_logs_in_again_with_the_new_one() {
    use wonton_client::{ChatSync, MessageSummary};

    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    let (_, device) = phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");

    // 隧道断掉：旧代理已经拆了，同步报“连不上”（断得快时先看到的可能是断开）。
    home.stop_tunnel().await;
    phone
        .wait_chat("连接断掉", |view| {
            (matches!(view.account, ChatAccount::SignedOut)
                || matches!(
                    &view.sync,
                    ChatSync::Failed {
                        reason: ChatFailure::TransportUnavailable,
                        ..
                    }
                ))
            .then_some(())
        })
        .await;

    // 隧道在新地址回来；旧代理上的同步再失败一次，就该拿新代理重新登录。
    home.start_tunnel().await;
    // Bot 直连核心，不经隧道：这条消息只能靠手机重新登录、重新同步收进来。
    bot_room
        .send_text("换了地址以后发的", &[])
        .await
        .expect("Bot 发消息");
    phone
        .wait_chat("经新地址收到消息", |view| {
            view.rooms
                .iter()
                .any(|row| {
                    row.room_id == room_id.as_str()
                        && row.preview.as_ref().is_some_and(|preview| {
                            preview.body
                                == MessageSummary::Text {
                                    text: "换了地址以后发的".to_owned(),
                                }
                        })
                })
                .then_some(())
        })
        .await;
    assert_eq!(
        phone.signed_in().await.1,
        device,
        "应还是原来的设备，不是新设备"
    );
    assert_eq!(home.login_tokens(), 1, "恢复会话不该再要登录令牌");

    // 不来回重登：安稳一阵以后还是同一个设备、还登录着。
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    assert_eq!(phone.signed_in().await.1, device);
}
