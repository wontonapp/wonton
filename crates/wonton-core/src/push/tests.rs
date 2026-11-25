//! 推送代理的行为测试：推送服务的替身是一个真实的 HTTP 服务（只在测试里），用设备的私钥解开它收到的推送。

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, StatusCode};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iroh::SecretKey;
use serde_json::json;
use tokio::net::TcpListener;
use web_push_native::{Auth, p256};

use super::*;
use wonton_protocol::{DevicePayload, MAX_BODY_CHARS, PushMessageType};

/// RFC 8291 附录 A 里设备（用户代理）的密钥。
const UA_PRIVATE: &str = "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94";
const UA_PUBLIC: &str =
    "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";

/// Tuwunel 完整格式里带着的正文：进推送明文，推送服务只看到密文。
const SECRET_BODY: &str = "今晚的会议改到八点";

struct Received {
    uri: String,
    headers: HeaderMap,
    body: Bytes,
}

/// 推送服务的替身：记下收到的请求，按 `status` 回话。
struct PushService {
    addr: SocketAddr,
    received: Arc<Mutex<Vec<Received>>>,
    status: Arc<Mutex<StatusCode>>,
}

#[derive(Clone)]
struct Recorder {
    received: Arc<Mutex<Vec<Received>>>,
    status: Arc<Mutex<StatusCode>>,
}

async fn record(
    State(recorder): State<Recorder>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    recorder.received.lock().unwrap().push(Received {
        uri: uri.to_string(),
        headers,
        body,
    });
    *recorder.status.lock().unwrap()
}

impl PushService {
    async fn start() -> Self {
        let recorder = Recorder {
            received: Arc::default(),
            status: Arc::new(Mutex::new(StatusCode::CREATED)),
        };
        let app = Router::new().fallback(record).with_state(recorder.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            addr,
            received: recorder.received,
            status: recorder.status,
        }
    }

    fn endpoint(&self) -> String {
        format!("http://{}/upWonton123?up=1", self.addr)
    }

    fn answer(&self, status: StatusCode) {
        *self.status.lock().unwrap() = status;
    }

    fn take(&self) -> Vec<Received> {
        std::mem::take(&mut *self.received.lock().unwrap())
    }
}

/// 一台已配对、已登记推送的设备，和它的推送服务。
struct Home {
    push: Push,
    server: EndpointId,
    devices: Arc<Devices>,
    device: EndpointId,
    pushkey: String,
    service: PushService,
    dir: tempfile::TempDir,
}

impl Home {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let devices = Arc::new(Devices::open(dir.path().join("devices.json")).unwrap());
        let device = SecretKey::generate().public();
        devices.add(device, None).unwrap();
        let server = SecretKey::generate().public();
        let push = Push::open(dir.path().join("push.json"), devices.clone(), server, None).unwrap();
        let service = PushService::start().await;
        let subscription = Subscription::new(&service.endpoint(), UA_PUBLIC, AUTH).unwrap();
        let pushkey = push.register(device, subscription).unwrap().pushkey;
        Self {
            push,
            server,
            devices,
            device,
            pushkey,
            service,
            dir,
        }
    }
}

/// Tuwunel 完整格式的一条通知（ruma 的 `send_event_notification`）。
fn notification(pushkey: &str) -> Notification {
    let request: NotifyRequest = serde_json::from_value(json!({
        "notification": {
            "event_id": "$reply-1", "room_id": "!bot:wonton.localhost",
            "type": "m.room.message", "sender": "@hermes:wonton.localhost",
            "sender_display_name": "Hermes", "room_name": "Hermes 主房间", "prio": "high",
            "content": {"msgtype": "m.text", "body": SECRET_BODY},
            "counts": {"unread": 1},
            "devices": [{"app_id": "app.wonton.android", "pushkey": pushkey, "pushkey_ts": 1, "data": {}}],
        }
    }))
    .unwrap();
    request.notification
}

/// 设备用自己的私钥和认证密钥解开推送，和 UnifiedPush 连接库做的一样。
fn decrypt(body: &[u8]) -> DevicePayload {
    let secret = p256::SecretKey::from_slice(&URL_SAFE_NO_PAD.decode(UA_PRIVATE).unwrap()).unwrap();
    let auth = Auth::clone_from_slice(&URL_SAFE_NO_PAD.decode(AUTH).unwrap());
    let plaintext = web_push_native::decrypt(body.to_vec(), &secret, &auth).unwrap();
    serde_json::from_slice(&plaintext).unwrap()
}

/// RFC 8291 第 5 节和附录 A 的测试向量：核心用的实现要能按标准加密，标准加密的（RFC 里那份固定的
/// 密文）也要解得开。Swift 通知扩展解的是同一种格式，它的单测用同一组向量。
#[test]
fn rfc_8291_vector_round_trips() {
    let plaintext = b"When I grow up, I want to be a watermelon";
    let ua_public =
        p256::PublicKey::from_sec1_bytes(&URL_SAFE_NO_PAD.decode(UA_PUBLIC).unwrap()).unwrap();
    let auth = Auth::clone_from_slice(&URL_SAFE_NO_PAD.decode(AUTH).unwrap());

    let body = web_push_native::encrypt(plaintext.to_vec(), &ua_public, &auth).unwrap();
    let secret = p256::SecretKey::from_slice(&URL_SAFE_NO_PAD.decode(UA_PRIVATE).unwrap()).unwrap();
    assert_eq!(
        web_push_native::decrypt(body, &secret, &auth).unwrap(),
        plaintext.to_vec()
    );

    let rfc = URL_SAFE_NO_PAD
        .decode(
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAm\
             S6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_\
             Qulcy4a-fN",
        )
        .unwrap();
    // 密文的 keyid 是应用服务器的公钥，收方（设备）用自己的私钥解：和上面同一把。
    assert_eq!(
        web_push_native::decrypt(rfc, &secret, &auth).unwrap(),
        plaintext.to_vec()
    );
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[tokio::test]
async fn a_paired_device_gets_the_body_encrypted_and_the_push_service_does_not() {
    let home = Home::start().await;

    let rejected = home
        .push
        .notify(&notification(&home.pushkey))
        .await
        .unwrap();
    assert!(rejected.is_empty());

    let received = home.service.take();
    assert_eq!(received.len(), 1);
    let push = &received[0];
    assert_eq!(push.uri, "/upWonton123?up=1");
    assert_eq!(push.headers["content-encoding"], "aes128gcm");
    assert_eq!(push.headers["ttl"], "86400");
    assert_eq!(push.headers["urgency"], "high");
    // 推送服务看到的只有密文：房间、事件、发送者、正文、类型都看不出来。
    for plaintext in [
        "!bot:wonton.localhost",
        "$reply-1",
        "@hermes",
        "Hermes",
        "Hermes 主房间",
        SECRET_BODY,
        "\"msgtype\":\"text\"",
    ] {
        assert!(
            !contains(&push.body, plaintext),
            "推送服务收到了明文 {plaintext}"
        );
    }
    assert_eq!(
        decrypt(&push.body),
        DevicePayload {
            server: home.server.to_string(),
            room_id: "!bot:wonton.localhost".into(),
            event_id: "$reply-1".into(),
            sender: Some("@hermes:wonton.localhost".into()),
            sender_display_name: Some("Hermes".into()),
            room_name: Some("Hermes 主房间".into()),
            prio: Some("high".into()),
            body: Some(SECRET_BODY.into()),
            msgtype: Some(PushMessageType::Text),
            truncated: false,
            thread_root: None,
            contact_request: None,
        },
        "设备解开后拿到正文和类型"
    );
    assert!(push.body.len() <= 4_096, "密文 {} 字节", push.body.len());
}

/// 任意事件类型和内容的一条通知。
fn notification_with(pushkey: &str, event_type: &str, content: serde_json::Value) -> Notification {
    let request: NotifyRequest = serde_json::from_value(json!({
        "notification": {
            "event_id": "$reply-1", "room_id": "!bot:wonton.localhost",
            "type": event_type, "sender": "@hermes:wonton.localhost",
            "sender_display_name": "Hermes", "room_name": "Hermes 主房间",
            "content": content,
            "devices": [{"app_id": "app.wonton.android", "pushkey": pushkey}],
        }
    }))
    .unwrap();
    request.notification
}

/// 长正文只放开头 120 个字符，标记已截断；加密后的请求仍在 4 KB 以内。
#[tokio::test]
async fn a_long_body_is_cut_and_marked_truncated() {
    let home = Home::start().await;
    let long = "今晚的会议改到八点，".repeat(60);
    let notice = notification_with(
        &home.pushkey,
        "m.room.message",
        json!({"msgtype": "m.text", "body": long}),
    );
    home.push.notify(&notice).await.unwrap();

    let received = home.service.take();
    let plaintext = decrypt(&received[0].body);
    let body = plaintext.body.unwrap();
    assert_eq!(body.chars().count(), MAX_BODY_CHARS);
    assert!(long.starts_with(body.as_str()));
    assert!(plaintext.truncated);
    assert_eq!(plaintext.msgtype, Some(PushMessageType::Text));
    assert!(received[0].body.len() <= 4_096);
    assert!(!contains(&received[0].body, &body));
}

/// 房间名和发送者显示名太长时先丢它们，正文和固定字段留着，密文仍在 4 KB 以内。
#[tokio::test]
async fn oversized_names_are_dropped_before_the_body() {
    let home = Home::start().await;
    let mut notice = notification(&home.pushkey);
    notice.room_name = Some("房".repeat(4_000));
    notice.sender_display_name = Some("名".repeat(4_000));
    home.push.notify(&notice).await.unwrap();

    let received = home.service.take();
    let plaintext = decrypt(&received[0].body);
    assert_eq!(plaintext.room_name, None);
    assert_eq!(plaintext.sender_display_name, None);
    assert_eq!(plaintext.body.as_deref(), Some(SECRET_BODY));
    assert!(!plaintext.truncated, "正文没动过");
    assert!(received[0].body.len() <= 4_096);
}

/// 附件只带类型和文件名，不带 URL 和缩略图。
#[tokio::test]
async fn an_attachment_push_carries_the_type_and_file_name_only() {
    let home = Home::start().await;
    let notice = notification_with(
        &home.pushkey,
        "m.room.message",
        json!({
            "msgtype": "m.image", "body": "猫.jpg", "filename": "猫.jpg",
            "url": "mxc://wonton.localhost/abc", "info": {"w": 10, "h": 10},
        }),
    );
    home.push.notify(&notice).await.unwrap();

    let received = home.service.take();
    let plaintext = decrypt(&received[0].body);
    assert_eq!(plaintext.msgtype, Some(PushMessageType::Image));
    assert_eq!(plaintext.body.as_deref(), Some("猫.jpg"));
    assert!(!contains(&received[0].body, "mxc://"));
    assert!(!contains(&received[0].body, "猫.jpg"));
}

/// 加密房间的通知只有密文：没有正文和类型，手机取回后再解密。
#[tokio::test]
async fn an_encrypted_event_is_only_a_wake_signal() {
    let home = Home::start().await;
    let notice = notification_with(
        &home.pushkey,
        "m.room.encrypted",
        json!({"ciphertext": {"a": "aes-sha2", "body": "密文正文"}}),
    );
    home.push.notify(&notice).await.unwrap();

    let received = home.service.take();
    let plaintext = decrypt(&received[0].body);
    assert_eq!(plaintext.body, None);
    assert_eq!(plaintext.msgtype, None);
    assert!(!contains(&received[0].body, "密文正文"));
}

#[tokio::test]
async fn unknown_pushkeys_and_revoked_devices_are_rejected_without_pushing() {
    let home = Home::start().await;

    let unknown = notification("not-a-registered-pushkey");
    assert_eq!(
        home.push.notify(&unknown).await.unwrap(),
        ["not-a-registered-pushkey"]
    );

    home.devices.remove(&home.device).unwrap();
    assert_eq!(
        home.push
            .notify(&notification(&home.pushkey))
            .await
            .unwrap(),
        std::slice::from_ref(&home.pushkey),
        "已吊销的设备：回 rejected，Tuwunel 随之删掉 pusher"
    );
    assert!(home.push.registration(&home.device).is_none(), "登记也删了");
    assert!(home.service.take().is_empty(), "没有发出任何推送");
}

#[tokio::test]
async fn a_gone_subscription_is_rejected_and_forgotten() {
    let home = Home::start().await;
    home.service.answer(StatusCode::GONE);

    let rejected = home
        .push
        .notify(&notification(&home.pushkey))
        .await
        .unwrap();

    assert_eq!(rejected, std::slice::from_ref(&home.pushkey));
    assert!(home.push.registration(&home.device).is_none());
}

#[tokio::test]
async fn a_struggling_push_service_makes_tuwunel_retry_and_a_refusal_does_not() {
    let home = Home::start().await;
    let notice = notification(&home.pushkey);

    home.service.answer(StatusCode::SERVICE_UNAVAILABLE);
    assert!(matches!(
        home.push.notify(&notice).await,
        Err(PushError::Unavailable(_))
    ));
    home.service.answer(StatusCode::TOO_MANY_REQUESTS);
    assert!(matches!(
        home.push.notify(&notice).await,
        Err(PushError::Unavailable(_))
    ));
    // 推送服务拒绝了这条（比如太大）：重试也没用，不让 Tuwunel 重发，订阅留着。
    home.service.answer(StatusCode::PAYLOAD_TOO_LARGE);
    assert!(home.push.notify(&notice).await.unwrap().is_empty());
    assert!(home.push.registration(&home.device).is_some());

    // 推送服务整个连不上：0 号端口上不会有人监听，连接立刻失败。先绑后关的随机端口可能被别的程序占去。
    let endpoint = "http://127.0.0.1:0/up";
    let subscription = Subscription::new(endpoint, UA_PUBLIC, AUTH).unwrap();
    home.push.register(home.device, subscription).unwrap();
    assert!(matches!(
        home.push.notify(&notice).await,
        Err(PushError::Unavailable(_))
    ));
}

#[tokio::test]
async fn counts_only_notifications_are_not_forwarded() {
    let home = Home::start().await;
    let request: NotifyRequest = serde_json::from_value(json!({
        "notification": {
            "counts": {"unread": 0},
            "devices": [{"app_id": "app.wonton.android", "pushkey": home.pushkey}],
        }
    }))
    .unwrap();

    assert!(
        home.push
            .notify(&request.notification)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(home.service.take().is_empty());
}

#[tokio::test]
async fn registrations_survive_a_restart_and_keep_their_pushkey() {
    let home = Home::start().await;
    let moved = Subscription::new("https://ntfy.example/upMoved?up=1", UA_PUBLIC, AUTH).unwrap();
    let again = home.push.register(home.device, moved.clone()).unwrap();
    assert_eq!(again.pushkey, home.pushkey, "换了推送地址，pushkey 不变");

    let path = home.dir.path().join("push.json");
    let reopened = Push::open(&path, home.devices.clone(), home.server, None).unwrap();
    let registration = reopened.registration(&home.device).unwrap();
    assert_eq!(registration.pushkey, home.pushkey);
    assert_eq!(registration.subscription, moved);
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "认证密钥在里面，只给核心读");
    let printed = format!("{registration:?}");
    assert!(
        !printed.contains(AUTH) && !printed.contains("upMoved") && !printed.contains(&home.pushkey),
        "Debug 不能带出认证密钥、推送地址的路径和 pushkey：{printed}"
    );

    assert!(reopened.unregister(&home.device).unwrap());
    assert!(!reopened.unregister(&home.device).unwrap());
    let restarted = Push::open(&path, home.devices.clone(), home.server, None).unwrap();
    assert!(restarted.registration(&home.device).is_none());
}

#[test]
fn invalid_subscriptions_are_refused() {
    let compressed = {
        let mut key = URL_SAFE_NO_PAD.decode(UA_PUBLIC).unwrap();
        key[0] = 0x02;
        URL_SAFE_NO_PAD.encode(key)
    };
    let off_curve = URL_SAFE_NO_PAD.encode([&[0x04u8][..], &[0u8; 64]].concat());
    let short_auth = URL_SAFE_NO_PAD.encode([0u8; 15]);
    for (endpoint, p256dh, auth) in [
        ("ftp://ntfy.example/up", UA_PUBLIC, AUTH),
        ("not a url", UA_PUBLIC, AUTH),
        ("https://ntfy.example/up", compressed.as_str(), AUTH),
        ("https://ntfy.example/up", off_curve.as_str(), AUTH),
        ("https://ntfy.example/up", "!!!", AUTH),
        ("https://ntfy.example/up", UA_PUBLIC, short_auth.as_str()),
    ] {
        assert!(
            matches!(
                Subscription::new(endpoint, p256dh, auth),
                Err(PushError::Invalid(_))
            ),
            "{endpoint} {p256dh} {auth}"
        );
    }
    let padded = format!("{AUTH}==");
    assert!(
        Subscription::new("https://ntfy.example/up", UA_PUBLIC, &padded).is_ok(),
        "带 = 填充的 base64url 也接受"
    );
}

/// 设备用自己的私钥解开推送，拿到原始明文（确认推送不是房间消息，按 JSON 看）。
fn decrypt_raw(body: &[u8]) -> serde_json::Value {
    let secret = p256::SecretKey::from_slice(&URL_SAFE_NO_PAD.decode(UA_PRIVATE).unwrap()).unwrap();
    let auth = Auth::clone_from_slice(&URL_SAFE_NO_PAD.decode(AUTH).unwrap());
    let plaintext = web_push_native::decrypt(body.to_vec(), &secret, &auth).unwrap();
    serde_json::from_slice(&plaintext).unwrap()
}

#[tokio::test]
async fn a_confirmation_push_reaches_only_the_recipients_devices() {
    let dir = tempfile::tempdir().unwrap();
    let devices = Arc::new(Devices::open(dir.path().join("devices.json")).unwrap());
    let owners = SecretKey::generate().public();
    devices.add(owners, None).unwrap();
    let alices = SecretKey::generate().public();
    devices
        .add_if(alices, None, Some("alice".into()), |_| true)
        .unwrap();
    let server = SecretKey::generate().public();
    let push = Push::open(dir.path().join("push.json"), devices, server, None).unwrap();
    // 两台设备各对一个推送服务：收到请求的端口说明是哪台设备收到的。
    let for_owner = PushService::start().await;
    let for_alice = PushService::start().await;
    push.register(
        owners,
        Subscription::new(&for_owner.endpoint(), UA_PUBLIC, AUTH).unwrap(),
    )
    .unwrap();
    push.register(
        alices,
        Subscription::new(&for_alice.endpoint(), UA_PUBLIC, AUTH).unwrap(),
    )
    .unwrap();

    let card = DeviceConfirmationPush {
        id: "card-1".into(),
        title: "claw 想安装 Grafana".into(),
        expires_at_ms: 42,
        origin: None,
        choices: Vec::new(),
    };
    let sent = push.send_confirmation(Some("alice"), &card).await.unwrap();
    assert_eq!(sent, 1, "只发给 alice 的设备");
    assert!(for_owner.take().is_empty(), "主人的设备不收别人的卡片");
    let received = for_alice.take();
    assert_eq!(received.len(), 1);
    // 明文里是 `confirmation` 段和服务器的 endpoint ID，设备凭它弹通知、点开去对的会话（ADR-0016）。
    assert_eq!(
        decrypt_raw(&received[0].body),
        json!({
            "server": server.to_string(),
            "confirmation": {"id": "card-1", "title": "claw 想安装 Grafana", "expires_at_ms": 42},
        }),
    );

    // 名下没有设备的人收不到：如实地失败，不假装发出去了。
    let error = push
        .send_confirmation(Some("nobody"), &card)
        .await
        .unwrap_err();
    assert!(matches!(error, PushError::Invalid(_)), "{error}");
    assert!(for_alice.take().is_empty());

    // 主人的卡片（收件人为空）只到主人的设备。
    assert_eq!(push.send_confirmation(None, &card).await.unwrap(), 1);
    assert_eq!(for_owner.take().len(), 1);
    assert!(for_alice.take().is_empty(), "家人的设备不收主人的卡片");
}

/// 收件人名下没有任何已配对设备时开卡：没人能答，调用方拿到 `confirmation_unavailable`，卡片不留在列表里
/// （ADR-0016）。
#[tokio::test]
async fn without_a_paired_device_nobody_can_confirm() {
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let devices = Arc::new(Devices::open(dir.path().join("devices.json")).unwrap());
    let server = SecretKey::generate().public();
    let push =
        Arc::new(Push::open(dir.path().join("push.json"), devices.clone(), server, None).unwrap());
    let confirmations = crate::mcp::Confirmations::new(
        Arc::new(crate::mcp::DevicePush::new(push)),
        Duration::from_secs(5),
    );
    let caller = crate::plugin::PluginId::parse("app.example.notes").unwrap();
    let draft = || crate::mcp::Draft {
        title: "Hermes 想卸载 Grafana".into(),
        lines: Vec::new(),
        choices: Vec::new(),
        scope: None,
        plugin_id: None,
        secret_inputs: Vec::new(),
        settings: None,
        category: None,
        permissions: Vec::new(),
        devices: Vec::new(),
    };
    let refusal = confirmations.ask(&caller, draft()).await.err().unwrap();
    assert_eq!(refusal.kind(), "confirmation_unavailable");
    // 主人有设备了，发给没有设备的家人的卡片照样没人能答。
    devices.add(SecretKey::generate().public(), None).unwrap();
    let audience = crate::mcp::Audience {
        recipients: crate::mcp::Recipients::Keeper(Some("alice".into())),
        ..Default::default()
    };
    let refusal = confirmations
        .ask_as(&caller, draft(), audience)
        .await
        .err()
        .unwrap();
    assert_eq!(refusal.kind(), "confirmation_unavailable");
    assert!(confirmations.list(None).0.is_empty());
    assert!(confirmations.list(Some("alice")).0.is_empty());
}
