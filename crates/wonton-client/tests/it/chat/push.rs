//! 推送：手机把推送订阅登记给家里、注册 pusher；App 被杀后 Bot 回复，Tuwunel 推给推送代理；推送唤醒的进程
//! 一创建核心就把推送交给它，核心等会话从本机恢复，经隧道取回消息、在本机解密，给出通知（ADR-0003）。

use serde_json::{Value, json};
use wonton_client::{
    Action, ChatFailure, CoreError, PushContent, PushEndpoint, PushStatus, RoomMembership,
    SecretString,
};
use wonton_matrix::testkit::Received;

use super::accept_the_bots_invite;
use super::support::{Home, Phone};

/// RFC 8291 附录 A 里设备的公钥和认证密钥。家里的推送代理在这个测试里是替身，不加密，只看登记的内容。
const P256DH: &str =
    "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";
const ENDPOINT: &str = "https://ntfy.example/upWonton123?up=1";

pub(crate) fn endpoint() -> PushEndpoint {
    PushEndpoint {
        url: ENDPOINT.to_owned(),
        p256dh: P256DH.to_owned(),
        auth: SecretString::new(AUTH),
    }
}

/// 推送代理会转给设备的明文（字段同核心的 `DevicePayload`），从它收到的通知里取。
pub(crate) fn device_payload(received: &Received) -> Vec<u8> {
    let notification = &received.notification;
    let field = |name: &str| notification.get(name).cloned().unwrap_or(Value::Null);
    json!({
        "room_id": field("room_id"),
        "event_id": field("event_id"),
        "sender": field("sender"),
        "sender_display_name": field("sender_display_name"),
        "room_name": field("room_name"),
    })
    .to_string()
    .into_bytes()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_bot_reply_wakes_a_killed_app_and_shows_the_decrypted_text() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;

    // 地址或公钥不对的订阅，壳给的时候就被拒绝。
    let mut broken = endpoint();
    broken.p256dh = "not-a-key".to_owned();
    assert!(matches!(
        phone.dispatch(Action::SetPushEndpoint { endpoint: broken }),
        Err(CoreError::InvalidInput { .. })
    ));
    phone
        .dispatch(Action::SetPushEndpoint {
            endpoint: endpoint(),
        })
        .unwrap();
    phone
        .wait_chat("推送已登记", |view| {
            (view.push == PushStatus::Registered).then_some(())
        })
        .await;
    let (method, subscription) = home.push_requests()[0].clone();
    assert_eq!(method, "PUT");
    assert_eq!(
        subscription,
        json!({"endpoint": ENDPOINT, "keys": {"p256dh": P256DH, "auth": AUTH}}),
        "登记给家里的是 W3C 推送订阅的格式"
    );

    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&phone, &bot).await;

    // App 被杀，Bot 这时回复：房间密钥经 to-device 发给手机，手机还没收到。
    drop(phone);
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    bot_room.send_text("你的快递到了").await.expect("Bot 回复");
    let pushed = home
        .push
        .wait_for("推送代理收到 Bot 回复的通知", |received| {
            received.notification["type"] == "m.room.encrypted"
                && received.notification["devices"][0]["pushkey"] == home.pushkey()
        })
        .await;

    // 推送唤醒的进程：用同一个数据目录创建核心，马上把推送交给它。
    let woken = Phone::open(dir.path());
    let notification = woken
        .resolve_push(device_payload(&pushed))
        .await
        .expect("推送内容是对的")
        .expect("这条消息要通知");
    assert_eq!(notification.room_id, room_id.as_str());
    assert_eq!(notification.sender, "Hermes");
    assert_eq!(notification.room_name, None, "私聊，标题用发送者");
    assert_eq!(
        notification.content,
        PushContent::Message {
            body: "你的快递到了".to_owned()
        }
    );
    assert!(matches!(
        woken.resolve_push(b"not json".to_vec()).await,
        Err(CoreError::InvalidInput { .. })
    ));

    // 分发器注销了本 App：照实显示，家里的登记和 pusher 都删掉。
    woken.signed_in().await;
    woken
        .dispatch(Action::SetPushEndpoint {
            endpoint: endpoint(),
        })
        .unwrap();
    woken
        .wait_chat("重新登记", |view| {
            (view.push == PushStatus::Registered).then_some(())
        })
        .await;
    woken
        .dispatch(Action::PushUnavailable {
            message: "分发器注销了本 App".to_owned(),
        })
        .unwrap();
    assert_eq!(
        woken.chat().push,
        PushStatus::Unavailable {
            message: "分发器注销了本 App".to_owned()
        }
    );
    wait_for_unregistered(&home).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_push_that_cannot_be_fetched_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;

    // 推送说的消息服务器上没有（比如被伪造的推送）：不显示成正常的消息，照实说取不到。
    let forged = json!({
        "room_id": "!nowhere:wonton.localhost",
        "event_id": "$forged",
        "sender_display_name": "Hermes",
    });
    let notification = phone
        .resolve_push(forged.to_string().into_bytes())
        .await
        .expect("格式是对的")
        .expect("要告诉主人有推送");
    assert_eq!(notification.sender, "Hermes");
    assert!(
        matches!(
            notification.content,
            PushContent::Unavailable {
                reason: ChatFailure::NotReady | ChatFailure::Upstream,
                ..
            }
        ),
        "{:?}",
        notification.content
    );
}

/// 推送只带房间和事件 ID，用不着读预览：主人自己建的私聊和 Bot 的主房间一样端到端加密。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn direct_chats_the_owner_starts_are_encrypted() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;

    phone
        .dispatch(Action::StartDirectChat {
            user_id: bot.user_id().to_string(),
        })
        .unwrap();
    let room = phone
        .wait_chat("建好私聊", |view| {
            view.rooms
                .iter()
                .find(|row| row.is_direct && row.membership == RoomMembership::Joined)
                .cloned()
        })
        .await;
    assert!(room.is_encrypted, "{room:?}");
}

async fn wait_for_unregistered(home: &Home) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    while !home
        .push_requests()
        .iter()
        .any(|(method, _)| method == "DELETE")
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "家里的推送登记没被取消：{:?}",
            home.push_requests()
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
