//! 推送里带的正文直接写成通知，也先落进本机时间线；服务器的真事件到了只留一条。

use super::*;

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
    use super::super::timeline::Timeline;
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
