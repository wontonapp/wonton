//! 确认卡片（ADR-0016）：测试扮演核心账号，在和主人的私聊里发卡片、编辑卡片；手机是真实的客户端核心，断言时间线里
//! 卡片的显示和发给核心的回答。

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use wonton_client::{
    Action, Answer, ChatFailure, ConfirmationState, ConfirmationView, CoreError, MessageContent,
    PushContent, PushStatus, RoomMembership, TimelineContent, TimelineItem, WindowView,
};
use wonton_matrix::{ChatRoom, MatrixSession, OwnedEventId, OwnedRoomId, OwnedUserId};

use super::push::{device_payload, endpoint};
use super::support::{Home, OWNER, Phone, Timeline};

const CARD_FIELD: &str = "app.wonton.confirmation";

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_owner_answers_cards_and_sees_how_they_end() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let (core, room_id) = core_room(&phone, &home).await;
    let room = core.open_room(&room_id).await.expect("核心打开私聊");
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;

    let install = install_card();
    let posted = send(&room, content(&install)).await;
    let view = card_view(&mut timeline, "install", "卡片出现").await;
    assert_eq!(view.card.state, ConfirmationState::Pending);
    assert_eq!(view.card.title, "Hermes 想做一件事");
    assert_eq!(view.card.lines.len(), 2);
    assert!(view.card.lines[1].emphasis);
    let labels: Vec<_> = view.card.choices.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, ["安装", "不装"]);
    assert_eq!(view.sending, None);
    assert_eq!(view.problem, None);

    // 点了按钮，发给核心；发出去到回应之间按钮不能再点。
    home.hold_answers(true);
    answer(&phone, "install", Answer::Approve);
    until_card(&mut timeline, "install", "正在发", |view| {
        view.sending == Some(Answer::Approve)
    })
    .await;
    home.hold_answers(false);

    // 核心收下了，卡片的编辑还没到，这里已经不能再答。连点不会再发一次。
    let view = until_card(&mut timeline, "install", "核心收下回答", |view| {
        view.card.state == ConfirmationState::Running
    })
    .await;
    assert_eq!(view.sending, None);
    assert!(view.card.choices.is_empty());
    answer(&phone, "install", Answer::Approve);
    answer(&phone, "install", Answer::Reject);
    assert_eq!(
        home.confirmation_answers(),
        [("install".to_owned(), json!({"answer": "approve"}))]
    );

    // 卡片原地更新成结果，不算“已编辑”。
    edit(
        &room,
        &posted,
        &with_state(&install, json!({"type": "running"})),
    )
    .await;
    let done = json!({"type": "succeeded", "message": "装好了 Grafana"});
    edit(&room, &posted, &with_state(&install, done)).await;
    until_card(&mut timeline, "install", "执行完", |view| {
        view.card.state
            == ConfirmationState::Succeeded {
                message: "装好了 Grafana".to_owned(),
            }
    })
    .await;
    let edited = timeline
        .until("卡片那一条", |item| message_of(item, "install"))
        .await;
    assert!(!edited, "卡片的更新不算编辑");

    // 拒绝。
    send(&room, content(&card("uninstall", in_ten_minutes()))).await;
    card_view(&mut timeline, "uninstall", "第二张卡片").await;
    answer(&phone, "uninstall", Answer::Reject);
    until_card(&mut timeline, "uninstall", "拒绝了", |view| {
        view.card.state == ConfirmationState::Rejected && view.card.choices.is_empty()
    })
    .await;
    assert_eq!(
        home.confirmation_answers().last().unwrap(),
        &("uninstall".to_owned(), json!({"answer": "reject"}))
    );

    // 主人的另一台设备答的，发给插件后没等到结果：如实显示“不确定”。
    let tool = card("tool", in_ten_minutes());
    let posted = send(&room, content(&tool)).await;
    card_view(&mut timeline, "tool", "第三张卡片").await;
    let uncertain = json!({"type": "uncertain", "message": "连接断了，可能已经执行"});
    edit(&room, &posted, &with_state(&tool, uncertain)).await;
    until_card(&mut timeline, "tool", "结果不确定", |view| {
        view.card.state
            == ConfirmationState::Uncertain {
                message: "连接断了，可能已经执行".to_owned(),
            }
            && view.card.choices.is_empty()
    })
    .await;

    // 重启后卡片还是最后的状态，不会又冒出按钮。
    drop(timeline);
    drop(phone);
    let phone = Phone::open(dir.path());
    phone.signed_in().await;
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;
    until_card(&mut timeline, "install", "重启后的结果", |view| {
        matches!(view.card.state, ConfirmationState::Succeeded { .. })
            && view.card.choices.is_empty()
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn answers_that_do_not_land_and_cards_that_run_out_show_what_happened() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let (core, room_id) = core_room(&phone, &home).await;
    let room = core.open_room(&room_id).await.expect("核心打开私聊");
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;

    // 不认识的回答发不出去。
    assert!(matches!(
        phone.dispatch(Action::AnswerConfirmation {
            id: "x".to_owned(),
            answer: Answer::Unknown,
            choice_id: None,
        }),
        Err(CoreError::InvalidInput { .. })
    ));

    // 隧道断了：说明原因，按钮还在，再点一次就好。
    send(&room, content(&card("flaky", in_ten_minutes()))).await;
    card_view(&mut timeline, "flaky", "卡片出现").await;
    home.reply_to_next_answer(502, json!({}));
    answer(&phone, "flaky", Answer::Approve);
    let view = until_card(&mut timeline, "flaky", "没答上", |view| {
        view.problem.is_some()
    })
    .await;
    assert_eq!(
        view.problem.unwrap().reason,
        ChatFailure::TransportUnavailable
    );
    assert_eq!(view.card.choices.len(), 2);
    assert_eq!(view.card.state, ConfirmationState::Pending);
    answer(&phone, "flaky", Answer::Approve);
    until_card(&mut timeline, "flaky", "再点一次答上了", |view| {
        view.card.state == ConfirmationState::Running && view.problem.is_none()
    })
    .await;

    // 主人的另一台设备先答了：说明原因，按钮消失；核心的编辑到了以后只看结果。
    let answered = card("answered", in_ten_minutes());
    let posted = send(&room, content(&answered)).await;
    card_view(&mut timeline, "answered", "卡片出现").await;
    home.reply_to_next_answer(409, core_error("already_answered", "这个确认已经回答过了"));
    answer(&phone, "answered", Answer::Reject);
    let view = until_card(&mut timeline, "answered", "核心不收", |view| {
        view.problem.is_some()
    })
    .await;
    let problem = view.problem.unwrap();
    assert_eq!(problem.reason, ChatFailure::InvalidInput);
    assert!(problem.message.contains("回答过了"), "{}", problem.message);
    assert!(view.card.choices.is_empty());
    let done = json!({"type": "succeeded", "message": "装好了"});
    edit(&room, &posted, &with_state(&answered, done)).await;
    until_card(&mut timeline, "answered", "核心的编辑到了", |view| {
        matches!(view.card.state, ConfirmationState::Succeeded { .. }) && view.problem.is_none()
    })
    .await;

    // 核心重启过，没有这个确认了。
    send(&room, content(&card("forgotten", in_ten_minutes()))).await;
    card_view(&mut timeline, "forgotten", "卡片出现").await;
    home.reply_to_next_answer(404, core_error("not_found", "没有这个确认"));
    answer(&phone, "forgotten", Answer::Approve);
    let view = until_card(
        &mut timeline,
        "forgotten",
        "核心没有这个确认",
        |view| view.problem.is_some(),
    )
    .await;
    assert!(view.card.choices.is_empty());

    // 核心说过期了，编辑还没到。
    send(&room, content(&card("late", in_ten_minutes()))).await;
    card_view(&mut timeline, "late", "卡片出现").await;
    home.reply_to_next_answer(409, core_error("expired", "这个确认已经过期"));
    answer(&phone, "late", Answer::Approve);
    until_card(&mut timeline, "late", "核心说过期了", |view| {
        view.card.state == ConfirmationState::Expired && view.problem.is_none()
    })
    .await;

    // 期限到了，核心没有编辑（比如它重启过）：到点自己变成过期，不用等新消息。
    let soon = now_ms() + 5_000;
    send(&room, content(&card("soon", soon))).await;
    let view = card_view(&mut timeline, "soon", "快过期的卡片").await;
    assert_eq!(view.card.state, ConfirmationState::Pending, "测试太慢了");
    until_card(&mut timeline, "soon", "到期", |view| {
        view.card.state == ConfirmationState::Expired && view.card.choices.is_empty()
    })
    .await;
}

/// Bot 在自己的房间里发同样结构的内容，当普通消息显示，不出按钮。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn only_the_core_account_can_show_a_card() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = joined_direct_chat(&phone, &bot).await;
    let room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;

    send(&room, content(&card("forged", in_ten_minutes()))).await;
    timeline
        .until("Bot 的消息是普通文字", |item| match &item.content {
            TimelineContent::Message { message } => match &message.body {
                MessageContent::Text { body, .. } if body.contains("请在 Wonton App") => Some(()),
                MessageContent::Confirmation { .. } => panic!("Bot 发的不能是卡片"),
                _ => None,
            },
            _ => None,
        })
        .await;
}

/// 主人不开 App 也要知道有卡片在等。不响的两种靠 Tuwunel 真实的默认推送规则：后面那张卡片的推送到了，说明前面的已经判定过。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_waiting_card_wakes_a_killed_app_and_its_edits_stay_quiet() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    phone.use_unified_push().await;
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
    let (core, room_id) = core_room(&phone, &home).await;
    let room = core.open_room(&room_id).await.expect("核心打开私聊");
    drop(phone);

    let waiting = card("install", in_ten_minutes());
    let posted = send(&room, content(&waiting)).await;
    let pushed = home
        .push
        .wait_for("卡片的推送", |received| {
            received.notification["event_id"] == posted.as_str()
        })
        .await;

    edit(
        &room,
        &posted,
        &with_state(&waiting, json!({"type": "running"})),
    )
    .await;
    send(&room, json!({"msgtype": "m.notice", "body": "不打扰主人"})).await;
    let next = send(&room, content(&card("next", in_ten_minutes()))).await;
    home.push
        .wait_for("下一张卡片的推送", |received| {
            received.notification["event_id"] == next.as_str()
        })
        .await;
    let messages: Vec<_> = home
        .push
        .received()
        .into_iter()
        .filter(|received| received.notification["type"] == "m.room.message")
        .map(|received| received.notification["event_id"].clone())
        .collect();
    assert_eq!(messages, [json!(posted), json!(next)]);

    let woken = Phone::open(dir.path());
    let notification = woken
        .resolve_push(device_payload(&home, &pushed))
        .await
        .expect("推送内容是对的")
        .expect("卡片要通知");
    assert_eq!(notification.room_id, room_id.as_str());
    assert_eq!(notification.sender, "Wonton");
    let PushContent::Message { body } = notification.content else {
        panic!("通知要有卡片的文字：{:?}", notification.content);
    };
    assert_eq!(body, content(&waiting)["body"].as_str().unwrap());
    let card = notification.confirmation.expect("通知要带上卡片");
    assert_eq!(card.id, waiting["id"].as_str().unwrap());
    assert_eq!(Some(card.expires_at_ms), waiting["expires_at_ms"].as_u64());
}

/// Agent 的卡片上可以有多个同为同意的按钮：回答带上点的是哪一个，正在发时也分得清（ADR-0035）。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn buttons_with_the_same_answer_are_told_apart() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let (core, room_id) = core_room(&phone, &home).await;
    let room = core.open_room(&room_id).await.expect("核心打开私聊");
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;

    let mut agent = card("agent", in_ten_minutes());
    agent["choices"] = json!([
        {"answer": "approve", "label": "这次允许", "choice_id": "once"},
        {"answer": "approve", "label": "总是允许", "choice_id": "always"},
        {"answer": "reject", "label": "拒绝", "choice_id": "no"},
    ]);
    send(&room, content(&agent)).await;
    let view = card_view(&mut timeline, "agent", "卡片出现").await;
    assert_eq!(view.card.choices.len(), 3);

    home.hold_answers(true);
    phone
        .dispatch(Action::AnswerConfirmation {
            id: "agent".to_owned(),
            answer: Answer::Approve,
            choice_id: Some("always".to_owned()),
        })
        .unwrap();
    until_card(&mut timeline, "agent", "正在发", |view| {
        view.sending == Some(Answer::Approve) && view.sending_choice_id.as_deref() == Some("always")
    })
    .await;
    home.hold_answers(false);

    until_card(&mut timeline, "agent", "核心收下回答", |view| {
        view.card.state == ConfirmationState::Running && view.sending_choice_id.is_none()
    })
    .await;
    assert_eq!(
        home.confirmation_answers(),
        [(
            "agent".to_owned(),
            json!({"answer": "approve", "choice_id": "always"})
        )]
    );
}

/// 比这个 App 新的核心写的：多了一个按钮和一个字段，别的按钮照常能点。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_window_shows_until_it_ends_and_the_owner_can_stop_it() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let (core, room_id) = core_room(&phone, &home).await;
    let room = core.open_room(&room_id).await.expect("核心打开私聊");
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;
    let stop = |id: &str| {
        phone
            .dispatch(Action::StopConfirmationWindow { id: id.to_owned() })
            .unwrap();
    };

    let until = in_ten_minutes();
    let posted = send(&room, content(&window_card("open", until, json!(null)))).await;
    let view = card_view(&mut timeline, "open", "时段生效中").await;
    assert_eq!(
        view.window,
        Some(WindowView::Active {
            expires_at_ms: until,
            stopping: false,
            problem: None,
        })
    );

    // 隧道断了：说明原因，时段还在，再点一次就好。
    home.reply_to_next_answer(502, json!({}));
    stop("open");
    let view = until_card(&mut timeline, "open", "没停下来", |view| {
        matches!(
            &view.window,
            Some(WindowView::Active {
                problem: Some(_),
                ..
            })
        )
    })
    .await;
    let Some(WindowView::Active {
        problem: Some(problem),
        ..
    }) = view.window
    else {
        unreachable!()
    };
    assert_eq!(problem.reason, ChatFailure::TransportUnavailable);

    // 发出去到回应之间不能再点，连点不会再发一次。
    home.hold_answers(true);
    stop("open");
    until_card(&mut timeline, "open", "正在停", |view| {
        matches!(
            view.window,
            Some(WindowView::Active {
                stopping: true,
                problem: None,
                ..
            })
        )
    })
    .await;
    stop("open");
    home.hold_answers(false);

    // 核心收下了，卡片的编辑还没到，这里已经显示结束；编辑到了换成核心写的放行次数。
    until_card(&mut timeline, "open", "停了", |view| {
        matches!(view.window, Some(WindowView::Ended { .. }))
    })
    .await;
    assert_eq!(home.window_stops(), ["open", "open"]);
    stop("open");
    assert_eq!(home.window_stops().len(), 2);
    let mut ended = window_card("open", until, json!("stopped"));
    ended["window"]["allowed"] = json!(3);
    edit(&room, &posted, &ended).await;
    let view = until_card(&mut timeline, "open", "放行次数", |view| {
        view.card
            .window
            .as_ref()
            .is_some_and(|window| window.allowed == Some(3))
    })
    .await;
    let Some(WindowView::Ended { note }) = view.window else {
        panic!("时段应该结束了：{:?}", view.window);
    };
    assert!(note.contains('3'));

    // 核心说没有这个时段：已经结束了，不是错误。
    send(&room, content(&window_card("gone", until, json!(null)))).await;
    card_view(&mut timeline, "gone", "时段生效中").await;
    home.reply_to_next_answer(
        404,
        json!({"error": {"kind": "not_found", "message": "没有"}}),
    );
    stop("gone");
    until_card(&mut timeline, "gone", "已结束", |view| {
        matches!(view.window, Some(WindowView::Ended { .. }))
    })
    .await;

    // 核心停着编辑不了卡片：到期时间到了，这里也显示结束。
    let soon = now_ms() + 2_000;
    send(&room, content(&window_card("runs-out", soon, json!(null)))).await;
    until_card(&mut timeline, "runs-out", "到期", |view| {
        matches!(view.window, Some(WindowView::Ended { .. }))
    })
    .await;
}

fn window_card(id: &str, expires_at_ms: u64, ended: Value) -> Value {
    let mut card = card(id, in_ten_minutes());
    card["state"] = json!({"type": "succeeded", "message": "已执行"});
    card["choices"] = json!([]);
    card["window"] = json!({"expires_at_ms": expires_at_ms, "allowed": 0, "ended": ended});
    card
}

fn install_card() -> Value {
    let mut install = card("install", in_ten_minutes());
    install["lines"] = json!([
        {"text": "看板和告警", "emphasis": false},
        {"text": "能访问 Docker", "emphasis": true},
    ]);
    install["choices"] = json!([
        {"answer": "approve", "label": "安装"},
        {"answer": "ask_later", "label": "晚点再问"},
        {"answer": "reject", "label": "不装"},
    ]);
    install["later"] = json!({"at_ms": 1});
    install
}

pub(crate) async fn core_room(phone: &Phone, home: &Home) -> (MatrixSession, OwnedRoomId) {
    let core = home.bot("wonton-core", "Wonton").await;
    let room_id = joined_direct_chat(phone, &core).await;
    (core, room_id)
}

/// 不加密的私聊，主人接受邀请。
pub(crate) async fn joined_direct_chat(phone: &Phone, peer: &MatrixSession) -> OwnedRoomId {
    let owner: OwnedUserId = OWNER.parse().unwrap();
    let room_id = peer
        .create_direct_chat(&owner, false)
        .await
        .expect("发起私聊");
    phone
        .wait_chat("收到邀请", |view| {
            view.rooms
                .iter()
                .any(|row| {
                    row.room_id == room_id.as_str() && row.membership == RoomMembership::Invited
                })
                .then_some(())
        })
        .await;
    phone
        .dispatch(Action::JoinRoom {
            room_id: room_id.to_string(),
        })
        .unwrap();
    phone
        .wait_chat("加入私聊", |view| {
            view.rooms
                .iter()
                .any(|row| {
                    row.room_id == room_id.as_str() && row.membership == RoomMembership::Joined
                })
                .then_some(())
        })
        .await;
    room_id
}

pub(crate) fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

pub(crate) fn in_ten_minutes() -> u64 {
    now_ms() + 600_000
}

pub(crate) fn card(id: &str, expires_at_ms: u64) -> Value {
    json!({
        "id": id,
        "state": {"type": "pending"},
        "title": "Hermes 想做一件事",
        "lines": [],
        "choices": [
            {"answer": "approve", "label": "允许"},
            {"answer": "reject", "label": "拒绝"},
        ],
        "expires_at_ms": expires_at_ms,
    })
}

pub(crate) fn with_state(card: &Value, state: Value) -> Value {
    let mut card = card.clone();
    card["state"] = state;
    card
}

/// 同 `wonton-core` 的 `cards::content`。
pub(crate) fn content(card: &Value) -> Value {
    json!({
        "msgtype": "m.text",
        "body": format!("{}\n请在 Wonton App 里确认。", card["title"].as_str().unwrap()),
        CARD_FIELD: card,
    })
}

pub(crate) fn core_error(kind: &str, message: &str) -> Value {
    json!({"error": {"kind": kind, "message": message}})
}

pub(crate) async fn send(room: &ChatRoom, content: Value) -> OwnedEventId {
    room.send_raw_message(content).await.expect("发消息")
}

/// 同 `wonton-core` 的 `NoticeRoom::edit`。
pub(crate) async fn edit(room: &ChatRoom, event: &OwnedEventId, card: &Value) {
    let new_content = content(card);
    let mut edit = new_content.clone();
    edit["body"] = json!(format!("* {}", new_content["body"].as_str().unwrap()));
    edit["m.new_content"] = new_content;
    edit["m.relates_to"] = json!({"rel_type": "m.replace", "event_id": event});
    send(room, edit).await;
}

pub(crate) fn answer(phone: &Phone, id: &str, answer: Answer) {
    phone
        .dispatch(Action::AnswerConfirmation {
            id: id.to_owned(),
            answer,
            choice_id: None,
        })
        .unwrap();
}

pub(crate) fn view_of(item: &TimelineItem, id: &str) -> Option<ConfirmationView> {
    let TimelineContent::Message { message } = &item.content else {
        return None;
    };
    match &message.body {
        MessageContent::Confirmation { view } if view.card.id == id => Some(view.clone()),
        _ => None,
    }
}

/// 这张卡片那条消息的 `edited`。
fn message_of(item: &TimelineItem, id: &str) -> Option<bool> {
    view_of(item, id)?;
    let TimelineContent::Message { message } = &item.content else {
        return None;
    };
    Some(message.edited)
}

async fn card_view(timeline: &mut Timeline, id: &str, what: &str) -> ConfirmationView {
    until_card(timeline, id, what, |_| true).await
}

pub(crate) async fn until_card(
    timeline: &mut Timeline,
    id: &str,
    what: &str,
    mut check: impl FnMut(&ConfirmationView) -> bool,
) -> ConfirmationView {
    timeline
        .until(what, |item| view_of(item, id).filter(|view| check(view)))
        .await
}
