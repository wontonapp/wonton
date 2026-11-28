//! 确认卡片的推送：凭卡片 ID 经单张接口取，通知带着来源会话；已经结束或取不到的照实说。

use super::*;

/// 确认卡片的推送（ADR-0016）：不是房间里的消息，只有卡片 ID；凭 ID 经单张接口取卡片，
/// 弹的通知带着来源会话，点开进那个房间。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_confirmation_push_shows_the_card_in_its_origin_room() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 这些测试只要推送取卡片（假核心 HTTP），不需要 Matrix：挡掉登录，省掉同步空转。
    home.fail_login_tokens(usize::MAX);
    phone.connect(&home);
    wait_connected(&phone).await;

    home.set_confirmations(
        vec![json!({
            "id": "install",
            "state": {"type": "pending"},
            "title": "Hermes 想安装 Grafana",
            "lines": [],
            "choices": [{"answer": "approve", "label": "安装"}],
            "expires_at_ms": (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64 + 600_000),
            "origin": {"room_id": "!hermes:wonton.localhost"},
        })],
        vec![],
    );
    let payload = json!({
        "server": home.server_id(),
        "confirmation": {
            "id": "install",
            "title": "Hermes 想安装 Grafana",
            "expires_at_ms": (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64 + 600_000),
            "origin": {"room_id": "!hermes:wonton.localhost"},
        },
    });
    let notification = phone
        .resolve_push(payload.to_string().into_bytes())
        .await
        .expect("推送内容是对的")
        .expect("在等回答的卡片要通知");
    assert_eq!(
        notification.category,
        wonton_client::NotificationCategory::Confirmation
    );
    assert_eq!(notification.room_id, "!hermes:wonton.localhost");
    assert_eq!(notification.event_id, "install");
    let confirmation = notification.confirmation.expect("带卡片");
    assert_eq!(confirmation.id, "install");
    assert_eq!(
        confirmation.origin.map(|origin| origin.room_id),
        Some("!hermes:wonton.localhost".to_owned())
    );
    assert_eq!(home.single_confirmation_requests(), 1);

    // 没有来源会话的卡片（核心自己发起的，比如服务器配对）：room_id 是空，点开进“等你确认”列表。
    home.set_confirmations(
        vec![json!({
            "id": "pairing",
            "state": {"type": "pending"},
            "title": "Alex 的服务器想和你的服务器连接",
            "lines": [],
            "choices": [{"answer": "approve", "label": "连接"}],
            "expires_at_ms": (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64 + 600_000),
        })],
        vec![],
    );
    let payload = json!({
        "server": home.server_id(),
        "confirmation": {
            "id": "pairing",
            "title": "Alex 的服务器想和你的服务器连接",
            "expires_at_ms": (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64 + 600_000),
        },
    });
    let notification = phone
        .resolve_push(payload.to_string().into_bytes())
        .await
        .expect("推送内容是对的")
        .expect("在等回答的卡片要通知");
    assert_eq!(notification.room_id, "", "没有来源会话，进列表");
}

/// 已经结束的卡片不弹；核心说没有的（结束太久、重启过忘了）也不弹。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn finished_cards_do_not_notify() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 这些测试只要推送取卡片（假核心 HTTP），不需要 Matrix：挡掉登录，省掉同步空转。
    home.fail_login_tokens(usize::MAX);
    phone.connect(&home);
    wait_connected(&phone).await;

    let payload = |id: &str| {
        json!({
            "server": home.server_id(),
            "confirmation": {"id": id, "title": "t", "expires_at_ms": (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64 + 600_000)},
        })
        .to_string()
        .into_bytes()
    };
    home.set_confirmations(
        vec![],
        vec![json!({
            "id": "install",
            "state": {"type": "succeeded", "message": "装好了"},
            "title": "t",
            "lines": [],
            "choices": [],
            "expires_at_ms": (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64 + 600_000),
        })],
    );
    let shown = phone
        .resolve_push(payload("install"))
        .await
        .expect("推送内容是对的");
    assert_eq!(shown, None, "已结束的不弹");

    home.hide_single_confirmation(true);
    let shown = phone
        .resolve_push(payload("gone"))
        .await
        .expect("推送内容是对的");
    assert_eq!(shown, None, "核心说没有的不弹");
    home.hide_single_confirmation(false);
}

/// 取不到卡片（核心出错、连接没就绪）：照实说取不到，用推送里带的标题。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_unfetchable_card_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 这些测试只要推送取卡片（假核心 HTTP），不需要 Matrix：挡掉登录，省掉同步空转。
    home.fail_login_tokens(usize::MAX);
    phone.connect(&home);
    wait_connected(&phone).await;

    let payload = json!({
        "server": home.server_id(),
        "confirmation": {"id": "install", "title": "Hermes 想安装 Grafana", "expires_at_ms": 1},
    })
    .to_string()
    .into_bytes();
    home.fail_single_confirmation(1);
    let notification = phone
        .resolve_push(payload.clone())
        .await
        .expect("推送内容是对的")
        .expect("取不到也照实说");
    assert!(
        matches!(
            notification.category,
            wonton_client::NotificationCategory::Confirmation
        ),
        "notification: {notification:?}"
    );
    match notification.content {
        PushContent::Unavailable { reason, message } => {
            assert_eq!(reason, ChatFailure::NotReady);
            assert_eq!(message, "Hermes 想安装 Grafana");
        }
        other => panic!("照实说取不到，结果是 {other:?}"),
    }
}
