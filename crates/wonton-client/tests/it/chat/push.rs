//! 推送：手机把推送订阅登记给家里、注册 pusher；App 被杀后 Bot 回复，Tuwunel 推给推送代理；推送唤醒的进程
//! 一创建核心就把推送交给它，核心等会话从本机恢复，经隧道取回消息（加密房间里的密文在本机解密），给出通知（ADR-0003）。

use std::time::Duration;

use serde_json::{Value, json};
use wonton_client::{
    Action, ChatFailure, CoreError, HomeStatus, MessageSummary, PushContent, PushEndpoint,
    PushStatus, RoomMembership, SecretString,
};
use wonton_matrix::testkit::Received;

use super::accept_the_bots_invite;
use super::support::{Home, Phone, WAIT};

mod body;
mod confirmation;
mod direct;
mod fetch;
mod other_servers;

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

/// `home` 的推送代理会转给设备的明文（字段同核心的 `DevicePayload`），从它收到的通知里取。
pub(crate) fn device_payload(home: &Home, received: &Received) -> Vec<u8> {
    let notification = &received.notification;
    let field = |name: &str| notification.get(name).cloned().unwrap_or(Value::Null);
    json!({
        "server": home.server_id(),
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

    phone.use_unified_push().await;

    // 地址或公钥不对的订阅，壳给的时候就被拒绝。
    let mut broken = endpoint();
    broken.p256dh = "not-a-key".to_owned();
    assert!(matches!(
        phone.dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint { endpoint: broken }
        }),
        Err(CoreError::InvalidInput { .. })
    ));
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: endpoint(),
            },
        })
        .unwrap();
    phone
        .wait_chat("推送已登记", |view| {
            (view.push == PushStatus::Registered).then_some(())
        })
        .await;
    // 开机按默认方式（保持连接）会清一次家里的登记，但 unregister 拿着锁等登录，醒来时壳可能已经切到
    // UnifiedPush 并给来了地址：这时直接登记新地址，不再发 DELETE。所以序列是 ["DELETE", "PUT"] 或只有
    // ["PUT"]，都对；PUT 之后再 DELETE 不会出现：清登记和登记共用一把锁，开机那次清登记先拿到、先做完。
    // `register` 在 PUT 返回、注册完 pusher 之后才发布 Registered，看到它时请求已经记全，不用再等。
    let requests = home.push_requests();
    let methods: Vec<_> = requests.iter().map(|(method, _)| method.as_str()).collect();
    assert!(
        matches!(methods.as_slice(), ["PUT"] | ["DELETE", "PUT"]),
        "家里的登记请求序列不对：{methods:?}"
    );
    assert_eq!(
        &requests.last().unwrap().1,
        &json!({"endpoint": ENDPOINT, "keys": {"p256dh": P256DH, "auth": AUTH}}),
        "登记给家里的是 W3C 推送订阅的格式"
    );

    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;

    // App 被杀，Bot 这时回复。
    drop(phone);
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    let sent = bot_room.send_text("你的快递到了", &[]).await;
    sent.expect("Bot 回复");
    let pushed = home
        .push
        .wait_for("推送代理收到 Bot 回复的通知", |received| {
            received.notification["type"] == "m.room.message"
                && received.notification["devices"][0]["pushkey"] == home.pushkey()
        })
        .await;

    // 推送唤醒的进程：用同一个数据目录创建核心，马上把推送交给它。
    let woken = Phone::open(dir.path());
    let notification = woken
        .resolve_push(device_payload(&home, &pushed))
        .await
        .expect("推送内容是对的")
        .expect("这条消息要通知");
    assert_eq!(notification.room_id, room_id.as_str());
    assert_eq!(notification.sender.as_deref(), Some("Hermes"));
    assert_eq!(notification.room_name, None, "私聊，标题用发送者");
    assert_eq!(
        notification.content,
        PushContent::Message {
            summary: MessageSummary::Text {
                text: "你的快递到了".to_owned()
            }
        }
    );
    assert!(matches!(
        woken.resolve_push(b"not json".to_vec()).await,
        Err(CoreError::InvalidInput { .. })
    ));

    // 分发器注销了本 App：照实显示，家里的登记和 pusher 都删掉。
    woken.signed_in().await;
    woken.use_unified_push().await;
    woken
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: endpoint(),
            },
        })
        .unwrap();
    woken
        .wait_chat("重新登记", |view| {
            (view.push == PushStatus::Registered).then_some(())
        })
        .await;
    woken
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::PushUnavailable {
                message: "分发器注销了本 App".to_owned(),
            },
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

/// 等和家里的连接就绪：推送路由按已登记的服务器行找作用域，行是连接成功时写下的。
pub(super) async fn wait_connected(phone: &Phone) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !matches!(phone.home().status, HomeStatus::Connected) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "和家里的连接 {WAIT:?} 内没有就绪"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
