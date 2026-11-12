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

/// RFC 8291 附录 A 里设备（用户代理）的密钥。
const UA_PRIVATE: &str = "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94";
const UA_PUBLIC: &str =
    "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";

/// Tuwunel 完整格式里带着的正文，不能出现在任何发出去的推送里。
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
        let push = Push::open(dir.path().join("push.json"), devices.clone()).unwrap();
        let service = PushService::start().await;
        let subscription = Subscription::new(&service.endpoint(), UA_PUBLIC, AUTH).unwrap();
        let pushkey = push.register(device, subscription).unwrap().pushkey;
        Self {
            push,
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

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[tokio::test]
async fn a_paired_device_gets_an_encrypted_notification_without_the_message_content() {
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
    // 推送服务看到的只有密文：房间、事件、发送者都看不出来。
    for plaintext in [
        "!bot:wonton.localhost",
        "$reply-1",
        "@hermes",
        "Hermes",
        SECRET_BODY,
    ] {
        assert!(
            !contains(&push.body, plaintext),
            "推送服务收到了明文 {plaintext}"
        );
    }
    assert_eq!(
        decrypt(&push.body),
        DevicePayload {
            room_id: "!bot:wonton.localhost".into(),
            event_id: "$reply-1".into(),
            sender: Some("@hermes:wonton.localhost".into()),
            sender_display_name: Some("Hermes".into()),
            room_name: Some("Hermes 主房间".into()),
            prio: Some("high".into()),
        },
        "设备解开后拿到房间和事件，没有正文"
    );
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
    let reopened = Push::open(&path, home.devices.clone()).unwrap();
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
    let restarted = Push::open(&path, home.devices.clone()).unwrap();
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
