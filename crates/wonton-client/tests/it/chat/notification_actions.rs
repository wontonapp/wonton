//! 通知动作、未读和锁屏隐私的核心行为回归。

use serde_json::json;
use wonton_client::{Action, CoreError, SettingsEntry};
use wonton_matrix::MessageBody;

use super::push::wait_connected;
use super::support::{Home, Phone, WAIT};
use super::{accept_the_bots_invite, bot_receives};
/// 无表单的卡片推送带按钮，通知上的“允许”能回答（ADR-0101 §1）：回答走核心的回答路径，核心收不到时
/// 调用方拿到错误，通知上要写得出原因。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_notification_button_answers_the_card_and_reports_failures() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    home.fail_login_tokens(usize::MAX);
    phone.connect(&home);
    wait_connected(&phone).await;

    let expires = now_ms() + 600_000;
    home.set_confirmations(
        vec![json!({
            "id": "install",
            "state": {"type": "pending"},
            "title": "Hermes 想安装 Grafana",
            "lines": [],
            "choices": [
                {"answer": "approve", "label": "安装"},
                {"answer": "reject", "label": "不装"},
            ],
            "expires_at_ms": expires,
        })],
        vec![],
    );
    let payload = json!({
        "server": home.server_id(),
        "confirmation": {"id": "install", "title": "Hermes 想安装 Grafana", "expires_at_ms": expires},
    })
    .to_string()
    .into_bytes();
    let notification = phone
        .resolve_push(payload)
        .await
        .expect("推送内容是对的")
        .expect("在等回答的卡片要通知");
    let confirmation = notification.confirmation.expect("带卡片");
    let labels: Vec<_> = confirmation
        .choices
        .iter()
        .map(|choice| choice.label.as_str())
        .collect();
    assert_eq!(labels, ["安装", "不装"], "通知上照着卡片给按钮");

    home.hold_answers(true);
    let answering = phone.answer_push_confirmation(
        notification.server_id.clone(),
        "install".into(),
        wonton_client::Answer::Approve,
        None,
    );
    tokio::pin!(answering);
    // 同一 future 已推进到 Sending，但核心没回：调用不能提前宣称成功。
    let views = phone.subscribe_confirmations();
    let wait_sending = async {
        loop {
            let view = views.next().await.unwrap();
            if view
                .pending
                .iter()
                .any(|card| card.card.id == "install" && card.sending.is_some())
            {
                break;
            }
        }
    };
    tokio::select! {
        result = &mut answering => panic!("returned before server accepted: {result:?}"),
        () = wait_sending => {}
    }
    let duplicate = phone
        .answer_push_confirmation(
            notification.server_id.clone(),
            "install".into(),
            wonton_client::Answer::Approve,
            None,
        )
        .await;
    assert!(matches!(duplicate, Err(CoreError::NotReady { .. })));
    home.hold_answers(false);
    answering.await.expect("核心收下了");
    let answers = home.confirmation_answers();
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].0, "install");
    assert_eq!(answers[0].1["answer"], "approve");

    assert_answer_failures(&phone, &home, notification.server_id, expires).await;
}

async fn assert_answer_failures(
    phone: &Phone,
    home: &Home,
    server_id: Option<String>,
    expires: u64,
) {
    // 核心回 500：调用方拿到错误（通知上写“没答上”），不假装成功。
    home.set_confirmations(
        vec![json!({
            "id": "windows",
            "state": {"type": "pending"},
            "title": "Hermes 想放行一小时",
            "lines": [],
            "choices": [{"answer": "approve", "label": "允许"}],
            "expires_at_ms": expires,
        })],
        vec![],
    );
    home.reply_to_next_answer(500, json!({"error": "服务内部错误"}));
    let failed = phone
        .answer_push_confirmation(
            server_id.clone(),
            "windows".to_owned(),
            wonton_client::Answer::Approve,
            None,
        )
        .await;
    assert!(
        matches!(failed, Err(CoreError::Upstream { .. })),
        "失败要报出来：{failed:?}"
    );

    home.reply_to_next_answer(
        403,
        json!({"error": {"kind": "forbidden", "message": "not allowed"}}),
    );
    let refused = phone
        .answer_push_confirmation(
            server_id.clone(),
            "windows".into(),
            wonton_client::Answer::Approve,
            None,
        )
        .await;
    assert!(matches!(refused, Err(CoreError::Unauthorized { .. })));
    let missing = phone
        .answer_push_confirmation(None, "install".into(), wonton_client::Answer::Approve, None)
        .await;
    assert!(matches!(missing, Err(CoreError::NotReady { .. })));
}

/// 通知里打的回复（ADR-0101 §1）：作为对那条消息的回复发出去，沿用发送文字的路。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_reply_typed_in_the_notification_reaches_the_room() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;

    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    bot_room.send_text("你的快递到了").await.expect("Bot 发送");
    let original = bot_receives(&bot_room, "Bot 看到自己发的消息", |message| {
        (message.event_id.is_some() && message.body == MessageBody::Text("你的快递到了".to_owned()))
            .then(|| message.clone())
    })
    .await;

    let target = wonton_protocol::TimelineTarget {
        room_id: room_id.to_string(),
        thread_root: None,
    };
    phone
        .dispatch(Action::Timeline {
            action: wonton_protocol::TimelineAction::SetDraft {
                target: target.clone(),
                body: "现在就下来".into(),
                reply_to: None,
            },
        })
        .unwrap();
    wait_for_saved_draft(&phone, target.clone()).await;
    drop(phone);
    let phone = Phone::open(dir.path());
    // 不先等 signed_in：通知动作自己等冷启动恢复，会话必须活到服务器接受发送。
    let original_id = original.event_id.clone().expect("服务器给了事件 ID");

    phone
        .reply_to_push(
            Some("1".into()),
            room_id.to_string(),
            None,
            Some(original_id.to_string()),
            " 现在就下来 ".to_owned(),
        )
        .await
        .expect("发出去了");
    let reply = bot_receives(
        &bot_room,
        "Bot 收到手机在通知里的回复",
        |message| {
            (message.body == MessageBody::Text("现在就下来".to_owned())).then(|| message.clone())
        },
    )
    .await;
    assert_eq!(
        reply.reply.map(|reply| reply.event_id),
        Some(original_id.clone()),
        "回复带着原消息"
    );
    phone
        .reply_to_push(
            Some("1".into()),
            room_id.to_string(),
            Some(original_id.to_string()),
            None,
            "旁聊回复".into(),
        )
        .await
        .unwrap();
    let threaded = bot_receives(&bot_room, "thread reply", |message| {
        (message.body == MessageBody::Text("旁聊回复".into())).then(|| message.clone())
    })
    .await;
    assert_eq!(threaded.thread_root.as_ref(), Some(&original_id));
    let mismatched = phone
        .reply_to_push(
            Some("1".into()),
            room_id.to_string(),
            Some("$wrong:example.org".into()),
            Some(original_id.to_string()),
            "不能错发".into(),
        )
        .await;
    assert!(matches!(mismatched, Err(CoreError::InvalidInput { .. })));
    assert_eq!(
        phone.draft(target).await.unwrap().unwrap().body,
        "现在就下来"
    );

    // 空白正文在通知上发不出去。
    assert!(matches!(
        phone
            .reply_to_push(
                Some("1".into()),
                room_id.to_string(),
                None,
                Some(original_id.to_string()),
                "  ".to_owned()
            )
            .await,
        Err(CoreError::InvalidInput { .. })
    ));
}

async fn wait_for_saved_draft(phone: &Phone, target: wonton_protocol::TimelineTarget) {
    tokio::time::timeout(WAIT, async {
        loop {
            if phone
                .draft(target.clone())
                .await
                .unwrap()
                .is_some_and(|draft| draft.body == "现在就下来")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

/// 锁屏隐私按类别（ADR-0101 §5）：关掉的类别在通知上标记隐藏，别的类别照旧；默认显示内容。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_lock_screen_switch_hides_only_the_category_it_is_set_for() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    home.fail_login_tokens(usize::MAX);
    phone.connect(&home);
    wait_connected(&phone).await;

    let expires = now_ms() + 600_000;
    home.set_confirmations(
        vec![json!({
            "id": "install",
            "state": {"type": "pending"},
            "title": "Hermes 想安装 Grafana",
            "lines": [],
            "choices": [{"answer": "approve", "label": "安装"}],
            "expires_at_ms": expires,
        })],
        vec![],
    );
    let payload = json!({
        "server": home.server_id(),
        "confirmation": {"id": "install", "title": "Hermes 想安装 Grafana", "expires_at_ms": expires},
    })
    .to_string()
    .into_bytes();
    let shown = phone
        .resolve_push(payload.clone())
        .await
        .unwrap()
        .expect("在等回答的卡片要通知");
    assert!(!shown.lock_screen_private, "默认显示内容");

    phone
        .set_lock_screen_content(wonton_client::NotificationCategory::Confirmation, true)
        .unwrap();
    let hidden = phone
        .resolve_push(payload.clone())
        .await
        .unwrap()
        .expect("在等回答的卡片要通知");
    assert!(hidden.lock_screen_private);
    // 设置页按类别显示这个开关，别的一类不动。
    let subscription = phone.subscribe_settings();
    let categories = tokio::time::timeout(WAIT, async {
        loop {
            let view = subscription.next().await.unwrap();
            let Some(SettingsEntry::Notifications { categories, .. }) = view
                .sections
                .iter()
                .flat_map(|section| section.entries.iter())
                .find(|entry| matches!(entry, SettingsEntry::Notifications { .. }))
            else {
                continue;
            };
            if categories
                .iter()
                .any(|category| category.lock_screen_private)
            {
                return categories.clone();
            }
        }
    })
    .await
    .expect("设置页跟着开关变");
    let private = |category| {
        categories
            .iter()
            .find(|row| row.category == category)
            .expect("每一类都有一行")
            .lock_screen_private
    };
    assert!(private(wonton_client::NotificationCategory::Confirmation));
    assert!(!private(wonton_client::NotificationCategory::Message));

    // 关回去，通知照旧显示内容。
    phone
        .set_lock_screen_content(wonton_client::NotificationCategory::Confirmation, false)
        .unwrap();
    let shown = phone
        .resolve_push(payload.clone())
        .await
        .unwrap()
        .expect("在等回答的卡片要通知");
    assert!(!shown.lock_screen_private);
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn cold_notification_answers_use_local_slots_and_never_fall_back_to_selected_server() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    let b = Home::start(&[phone.device_id()]).await;
    phone.connect(&a);
    wait_connected(&phone).await;
    let card = json!({"id": "cold", "state": {"type": "pending"}, "title": "confirm", "lines": [],
        "choices": [{"answer": "approve", "label": "allow"}], "expires_at_ms": now_ms() + 600_000});
    a.set_confirmations(vec![card.clone()], vec![]);
    b.set_confirmations(vec![card], vec![]);
    phone.dispatch(Action::AddServer).unwrap();
    phone.connect(&b);
    wait_connected(&phone).await;
    drop(phone);
    let phone = Phone::open(dir.path());
    phone
        .answer_push_confirmation(
            Some("1".into()),
            "cold".into(),
            wonton_client::Answer::Approve,
            None,
        )
        .await
        .unwrap();
    assert_eq!(a.confirmation_answers().len(), 1);
    assert!(b.confirmation_answers().is_empty());
    phone
        .dispatch(Action::RemoveServer { id: "1".into() })
        .unwrap();
    for server in [None, Some("1".to_owned()), Some(b.server_id())] {
        let result = phone
            .answer_push_confirmation(server, "cold".into(), wonton_client::Answer::Approve, None)
            .await;
        assert!(matches!(result, Err(CoreError::NotReady { .. })));
    }
    assert!(b.confirmation_answers().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn notification_actions_refuse_forms_and_report_refused_answers_in_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    home.fail_login_tokens(usize::MAX);
    phone.connect(&home);
    wait_connected(&phone).await;
    let card = json!({"id": "form", "state": {"type": "pending"}, "title": "settings", "lines": [],
        "choices": [{"answer": "approve", "label": "allow"}], "expires_at_ms": now_ms() + 600_000,
        "settings": {"fields": []}});
    // 有表单的卡片必须在 App 处理，即使字段列表为空也不能在通知批准。
    home.set_confirmations(vec![card], vec![]);
    let result = phone
        .answer_push_confirmation(
            Some("1".into()),
            "form".into(),
            wonton_client::Answer::Approve,
            None,
        )
        .await;
    assert!(result.is_err());
    assert!(home.confirmation_answers().is_empty());
    home.set_confirmations(
        vec![
            json!({"id": "refused", "state": {"type": "pending"}, "title": "confirm", "lines": [],
        "choices": [{"answer": "approve", "label": "allow"}], "expires_at_ms": now_ms() + 600_000}),
        ],
        vec![],
    );
    home.reply_to_next_answer(
        409,
        json!({"error": {"kind": "already_answered", "message": "answered"}}),
    );
    let result = phone
        .answer_push_confirmation(
            Some("1".into()),
            "refused".into(),
            wonton_client::Answer::Approve,
            None,
        )
        .await;
    assert!(matches!(result, Err(CoreError::InvalidInput { .. })));
    let subscription = phone.subscribe_confirmations();
    tokio::time::timeout(WAIT, async {
        loop {
            let view = subscription.next().await.unwrap();
            if view
                .pending
                .iter()
                .any(|card| card.card.id == "refused" && card.problem.is_some())
            {
                break;
            }
        }
    })
    .await
    .unwrap();
}
