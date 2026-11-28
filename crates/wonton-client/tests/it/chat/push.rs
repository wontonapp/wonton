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

mod confirmation;
mod direct;
mod fetch;

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

/// 推送来自不是当前连着的那台（ADR-0042）：临时连上那台取回、解密，当前这台不受影响。从手机上移除了那台以后，
/// 照实说是已移除的服务器发来的，不显示成消息。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_push_from_a_server_you_are_not_on_is_fetched_from_that_server() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    let b = Home::start(&[phone.device_id()]).await;
    phone.connect(&a);
    phone.signed_in().await;
    phone.use_unified_push().await;
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: endpoint(),
            },
        })
        .unwrap();
    let registered =
        |view: &wonton_client::ChatViewModel| (view.push == PushStatus::Registered).then_some(());
    phone.wait_chat("A 上推送已登记", registered).await;
    let bot = a.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&a, &phone, &bot).await;

    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AddServer,
        })
        .unwrap();
    phone.connect(&b);
    let (_, on_b) = phone.signed_in().await;
    phone.wait_chat("B 上推送已登记", registered).await;

    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    let sent = bot_room.send_text("A 上的新消息", &[]).await;
    sent.expect("Bot 回复");
    let pushed = a
        .push
        .wait_for("A 的推送代理收到通知", |received| {
            received.notification["type"] == "m.room.message"
                && received.notification["devices"][0]["pushkey"] == a.pushkey()
        })
        .await;
    let payload = device_payload(&a, &pushed);
    let notification = phone
        .resolve_push(payload.clone())
        .await
        .expect("推送内容是对的")
        .expect("这条消息要通知");
    assert_eq!(notification.server_id.as_deref(), Some("1"));
    assert_eq!(notification.room_id, room_id.as_str());
    assert_eq!(
        notification.content,
        PushContent::Message {
            summary: MessageSummary::Text {
                text: "A 上的新消息".to_owned()
            }
        }
    );
    assert_eq!(phone.signed_in().await.1, on_b, "当前还连着 B");
    assert_eq!(a.login_tokens(), 1, "临时连 A 用的是原来的会话");

    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::RemoveServer { id: "1".into() },
        })
        .unwrap();
    let notification = phone
        .resolve_push(payload)
        .await
        .expect("推送内容是对的")
        .expect("要告诉主人有推送");
    assert_eq!(notification.server_id, None);
    assert!(
        matches!(
            notification.content,
            PushContent::Unavailable {
                reason: ChatFailure::NotReady,
                ..
            }
        ),
        "{:?}",
        notification.content
    );
}

/// 推送地址变了（ADR-0042）：当前这台马上重新登记，配过的另一台在后台临时连上去登记。地址没变时（每次启动壳都会
/// 再给一遍）另一台不用再登记。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_new_push_endpoint_is_registered_on_the_other_servers_too() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    let b = Home::start(&[phone.device_id()]).await;
    let registered =
        |view: &wonton_client::ChatViewModel| (view.push == PushStatus::Registered).then_some(());
    phone.connect(&a);
    phone.signed_in().await;
    phone.use_unified_push().await;
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: endpoint(),
            },
        })
        .unwrap();
    phone.wait_chat("A 上推送已登记", registered).await;
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AddServer,
        })
        .unwrap();
    phone.connect(&b);
    phone.signed_in().await;
    phone.wait_chat("B 上推送已登记", registered).await;

    let mut moved = endpoint();
    moved.url = "https://ntfy.example/upWonton456?up=1".to_owned();
    phone.use_unified_push().await;
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: moved.clone(),
            },
        })
        .unwrap();
    phone.wait_chat("B 上用新地址登记", registered).await;
    let registered_on_a =
        async {
            while !a.push_requests().iter().any(|(method, subscription)| {
                method == "PUT" && subscription["endpoint"] == moved.url
            }) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
    tokio::time::timeout(Duration::from_secs(90), registered_on_a)
        .await
        .expect("A 应在后台用新地址重新登记");
    assert!(
        matches!(phone.home().status, HomeStatus::Connected) && phone.servers().servers[1].current,
        "当前还连着 B"
    );

    let on_a = a.push_requests().len();
    let on_b = b.push_requests().len();
    phone.use_unified_push().await;
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint { endpoint: moved },
        })
        .unwrap();
    phone.wait_chat("B 再登记一遍", registered).await;
    assert!(b.push_requests().len() > on_b);
    assert_eq!(a.push_requests().len(), on_a, "地址没变，A 不用再登记");
}

/// 等家里到达一条满足条件的推送登记请求；靠假核心的到达计数等事件，不睡固定间隔。
async fn wait_for_a_registration(
    home: &Home,
    within: Duration,
    mut found: impl FnMut(&[(String, Value)]) -> bool,
) -> bool {
    let mut seen = home.push_registrations_seen();
    let waiting = async {
        loop {
            if found(&home.push_requests()) {
                return true;
            }
            if seen.changed().await.is_err() {
                panic!("推送登记的假核心没了");
            }
        }
    };
    tokio::time::timeout(within, waiting).await.unwrap_or(false)
}

/// 另一台还在登记（家里已收到 PUT、登记还没走完）时，壳再给一遍同一个地址（Android 的 UnifiedPush 开机时常常
/// 连着回调几次 `onNewEndpoint`），家里只该收到一次 PUT。登记走完之前只有落盘记录能去重，而记录要等登记走完
/// 才写，所以核心得记着“正在给哪台登记哪个地址”。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_repeated_endpoint_while_another_server_is_still_registering_is_not_registered_again() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    let b = Home::start(&[phone.device_id()]).await;
    let registered =
        |view: &wonton_client::ChatViewModel| (view.push == PushStatus::Registered).then_some(());
    phone.connect(&a);
    phone.signed_in().await;
    phone.use_unified_push().await;
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: endpoint(),
            },
        })
        .unwrap();
    phone.wait_chat("A 上推送已登记", registered).await;
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AddServer,
        })
        .unwrap();
    phone.connect(&b);
    phone.signed_in().await;
    phone.wait_chat("B 上推送已登记", registered).await;

    // 挂起 A 的登记响应：家里已收到 PUT、登记停在等响应，正好是写盘前的窗口。
    a.hold_push_registrations(true);
    let mut moved = endpoint();
    moved.url = "https://ntfy.example/upWonton456?up=1".to_owned();
    let puts_with_moved = |requests: &[(String, Value)]| {
        requests
            .iter()
            .filter(|(m, s)| m == "PUT" && s["endpoint"] == moved.url)
            .count()
    };
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: moved.clone(),
            },
        })
        .unwrap();
    assert!(
        wait_for_a_registration(&a, WAIT, |requests| puts_with_moved(requests) > 0).await,
        "家里没等到 A 用新地址登记"
    );

    // 窗口里壳再给一遍同一个地址。放行让挂起的登记走完：重复的登记跟第一次共用同一把登记锁，
    // 它的 PUT 只会出现在放行之后，所以断言放在放行后、给第二次 PUT 一个有界的机会再数。
    phone
        .dispatch(Action::Push {
            action: wonton_protocol::PushAction::SetPushEndpoint {
                endpoint: moved.clone(),
            },
        })
        .unwrap();
    a.hold_push_registrations(false);
    let _ = wait_for_a_registration(&a, Duration::from_secs(2), |requests| {
        puts_with_moved(requests) > 1
    })
    .await;
    assert_eq!(
        puts_with_moved(&a.push_requests()),
        1,
        "同一个地址家里只该收到一次 PUT"
    );
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

/// 推送里带了正文（ADR-0118）：不用等隧道取消息，通知直接写出来；附件只写类型和文件名，截断的标出来。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_push_with_the_body_shows_it_without_fetching() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;

    // 房间里没有这条事件：正文只可能来自推送本身。取消息那条路会先等会话、再等 15 秒超时。
    let pushed = |body: &str, msgtype: &str, truncated: bool| {
        json!({
            "server": home.server_id(),
            "room_id": "!nowhere:wonton.localhost",
            "event_id": "$forged",
            "sender": "@hermes:wonton.localhost",
            "sender_display_name": "Hermes",
            "body": body,
            "msgtype": msgtype,
            "truncated": truncated,
        })
        .to_string()
        .into_bytes()
    };
    let started = std::time::Instant::now();
    let notification = phone
        .resolve_push(pushed("今晚的会议改到八点", "m.text", false))
        .await
        .expect("推送内容是对的")
        .expect("这条消息要通知");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "带正文的推送没去取消息"
    );
    assert_eq!(notification.sender.as_deref(), Some("Hermes"));
    assert_eq!(
        notification.content,
        PushContent::Message {
            summary: MessageSummary::Text {
                text: "今晚的会议改到八点".to_owned()
            }
        }
    );

    let image = phone
        .resolve_push(pushed("IMG_0001.jpg", "m.image", false))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        image.content,
        PushContent::Message {
            summary: MessageSummary::Text {
                text: "Photo IMG_0001.jpg".to_owned()
            }
        }
    );

    let cut = phone
        .resolve_push(pushed("开头一段", "m.text", true))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        cut.content,
        PushContent::Message {
            summary: MessageSummary::Text {
                text: "开头一段…".to_owned()
            }
        },
        "截断的标出来"
    );
}

/// 带正文的推送顺手进了本机时间线，点开房间就能看到；服务器的同一条消息到了只留一条。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_pushed_body_is_in_the_timeline_until_the_real_event_arrives() {
    use super::timeline::Timeline;
    use wonton_client::MessageContent;

    fn preview_text(item: &wonton_client::TimelineItem) -> Option<String> {
        let wonton_client::TimelineContent::Message { message } = &item.content else {
            return None;
        };
        match &message.body {
            MessageContent::Text { body, .. } => Some(body.clone()),
            _ => None,
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;

    // 默认的保持连接方式不注册 pusher：后面要等 Bot 那条真消息经 Tuwon 推到推送代理，得先切成 UnifiedPush 并登记。
    phone.use_unified_push().await;
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

    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;

    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;

    // 通知点开时这条消息还没同步：推送里的正文先显示出来。
    let early = format!("$preview-{}", wonton_matrix::testkit::random_hex(8));
    let payload = json!({
        "server": home.server_id(),
        "room_id": room_id,
        "event_id": early,
        "sender": "@hermes:wonton.localhost",
        "sender_display_name": "Hermes",
        "body": "先看到这一句",
        "msgtype": "m.text",
    })
    .to_string()
    .into_bytes();
    phone.resolve_push(payload).await.unwrap().unwrap();
    timeline
        .until("预览到了", |item| {
            (preview_text(item).as_deref() == Some("先看到这一句")).then_some(())
        })
        .await;
    assert_eq!(
        timeline
            .items
            .iter()
            .filter(|item| preview_text(item).as_deref() == Some("先看到这一句"))
            .count(),
        1
    );

    // 服务器的真消息同步到以后，同一个房间再开一条时间线：只留真正的事件，不再补预览。
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    bot_room
        .send_text("后到的真消息", &[])
        .await
        .expect("Bot 回复");
    let pushed = home
        .push
        .wait_for("真消息的推送", |received| {
            received.notification["content"]["body"] == "后到的真消息"
        })
        .await;
    let mut real = device_payload(&home, &pushed);
    let mut value: Value = serde_json::from_slice(&real).unwrap();
    value["body"] = "后到的真消息".into();
    real = serde_json::to_vec(&value).unwrap();
    phone.resolve_push(real).await.unwrap().unwrap();

    let mut reopened = Timeline::open(&phone, room_id.as_str()).await;
    reopened
        .until("真消息到了", |item| {
            (preview_text(item).as_deref() == Some("后到的真消息")).then_some(())
        })
        .await;
    assert_eq!(
        reopened
            .items
            .iter()
            .filter(|item| preview_text(item).as_deref() == Some("后到的真消息"))
            .count(),
        1,
        "真事件和预览只能留一条"
    );
}
