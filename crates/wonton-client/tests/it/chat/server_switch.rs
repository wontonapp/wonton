//! 同一台手机换服务器：每台服务器的 homeserver 地址和用户都一样，只有隧道的身份不同。

use std::time::Duration;

use wonton_client::{Action, ChatAccount, ChatFailure, ChatSync, MessageSummary};
use wonton_protocol::TimelineTarget;

use super::support::{Home, Phone};
use super::{accept_the_bots_invite, chat_with_the_bot};

/// 从 A 改配到 B：不拿 A 的令牌去 B（那样会显示“已被吊销”），登录成新设备，聊天能用，重启后还在 B 上。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn moving_to_another_server_signs_in_afresh() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    let b = Home::start(&[phone.device_id()]).await;
    phone.connect(&a);
    let (_, on_a) = phone.signed_in().await;
    let a_bot = a.bot("hermes", "Hermes").await;
    let a_room = accept_the_bots_invite(&a, &phone, &a_bot).await;

    phone.connect(&b);

    let on_b = phone
        .wait_chat("登录到 B", |view| {
            assert!(
                !matches!(
                    &view.account,
                    ChatAccount::Failed { reason, .. } if *reason == ChatFailure::Unauthorized
                ),
                "换了服务器不应说设备被吊销：{:?}",
                view.account
            );
            match (&view.account, &view.sync) {
                (ChatAccount::SignedIn { device_id, .. }, ChatSync::Running)
                    if b.login_tokens() == 1 =>
                {
                    Some(device_id.clone())
                }
                _ => None,
            }
        })
        .await;
    assert_ne!(on_b, on_a, "B 上应是一台新设备");
    assert!(
        phone
            .chat()
            .rooms
            .iter()
            .all(|row| row.room_id != a_room.as_str()),
        "B 上不应还留着 A 的房间"
    );
    let b_bot = b.bot("hermes", "Hermes").await;
    let b_room = accept_the_bots_invite(&b, &phone, &b_bot).await;
    chat_with_the_bot(&phone, &b_bot, &b_room).await;
    drop(phone);

    let phone = Phone::open(dir.path());
    let (_, restored) = phone.signed_in().await;
    assert_eq!(restored, on_b, "重启后应恢复 B 上的会话");
    assert_eq!(b.login_tokens(), 1, "恢复会话不应再要登录令牌");
    assert_eq!(a.login_tokens(), 1);
}

/// 同一台服务器换了地址：身份没变，接着用原来的设备和加密存储。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_new_address_for_the_same_server_keeps_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    let (_, device) = phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");

    // 旧地址已经不通，所以手机只能经新地址收到这条消息；预览能显示正文，说明房间密钥也在。
    home.move_tunnel().await;
    bot_room
        .send_text("换了地址之后发的")
        .await
        .expect("Bot 发消息");
    phone.connect(&home);

    phone
        .wait_chat("经新地址收到消息", |view| {
            view.rooms
                .iter()
                .any(|row| {
                    row.room_id == room_id.as_str()
                        && row.preview.as_ref().is_some_and(|preview| {
                            preview.body
                                == MessageSummary::Text {
                                    text: "换了地址之后发的".to_owned(),
                                }
                        })
                })
                .then_some(())
        })
        .await;
    assert_eq!(phone.signed_in().await.1, device, "应还是原来的设备");
    assert_eq!(home.login_tokens(), 1, "换地址不应重新登录");
}

/// 配了两台、切回第一台（ADR-0042）：A 的会话、房间和加密存储都还在，不用重新登录；切走时打开着的时间线结束。
/// 切换在没有 tokio 运行时的线程上发起，和壳一样（Android 的主线程）：旧作用域在那里释放也不能崩。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn switching_back_keeps_the_first_servers_chat() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    let b = Home::start(&[phone.device_id()]).await;
    phone.connect(&a);
    let (_, on_a) = phone.signed_in().await;
    let a_bot = a.bot("hermes", "Hermes").await;
    let a_room = accept_the_bots_invite(&a, &phone, &a_bot).await;
    let timeline = phone
        .subscribe_timeline(TimelineTarget {
            room_id: a_room.to_string(),
            thread_root: None,
        })
        .await
        .expect("打开 A 的房间");

    dispatch_from_shell(&phone, Action::AddServer);
    let ended = async { while timeline.next().await.is_some() {} };
    tokio::time::timeout(Duration::from_secs(30), ended)
        .await
        .expect("切走以后时间线应结束");
    phone.connect(&b);
    let (_, on_b) = phone.signed_in().await;
    assert_ne!(on_b, on_a);

    dispatch_from_shell(&phone, Action::SwitchServer { id: "1".into() });
    phone
        .wait_chat("切回 A", |view| {
            let signed_in = matches!(
                (&view.account, &view.sync),
                (ChatAccount::SignedIn { device_id, .. }, ChatSync::Running) if *device_id == on_a
            );
            let has_room = view.rooms.iter().any(|row| row.room_id == a_room.as_str());
            (signed_in && has_room).then_some(())
        })
        .await;
    assert_eq!(a.login_tokens(), 1, "切回来不应重新登录");
    chat_with_the_bot(&phone, &a_bot, &a_room).await;
}

/// 壳在自己的线程上调 `dispatch`，那里没有 tokio 运行时。
fn dispatch_from_shell(phone: &Phone, action: Action) {
    std::thread::scope(|scope| scope.spawn(|| phone.dispatch(action)).join())
        .expect("dispatch 不应 panic")
        .expect("dispatch 应成功");
}
