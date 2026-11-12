//! “等你确认”列表（ADR-0016）：客户端长轮询核心的确认接口，把等回答的、结束的卡片交给壳。
//! 测试扮演核心（假核心的 HTTP 接口），手机是真实的客户端核心。卡片不是 Matrix 消息，这些测试不用 Docker。

use serde_json::{Value, json};
use wonton_client::{
    Action, Answer, ChatFailure, ConfirmationState, ConfirmationsViewModel, CoreError,
};
use wonton_protocol::{TimelineAction, TimelineTarget};

use super::confirmation::{core_room, joined_direct_chat};

use super::support::{Home, Phone, WAIT};

fn card(id: &str, room: Option<&str>, expires_at_ms: u64) -> Value {
    let mut card = json!({
        "id": id,
        "state": {"type": "pending"},
        "title": format!("{id} 要确认"),
        "lines": [],
        "choices": [
            {"answer": "approve", "label": "允许"},
            {"answer": "reject", "label": "拒绝"},
        ],
        "expires_at_ms": expires_at_ms,
    });
    if let Some(room) = room {
        card["origin"] = json!({"room_id": room});
    }
    card
}

fn bot_room_card(id: &str, room: &str) -> Value {
    let mut card = card(id, Some(room), u64::MAX / 4);
    card["requester"] = json!({
        "type": "bot",
        "bot": {"plugin_id": "app.hermes", "bot": "owner"},
        "user_id": "@hermes:wonton.localhost",
    });
    card
}

/// 等待“等你确认”列表满足条件。
async fn wait_confirmations<R>(
    phone: &Phone,
    what: &str,
    mut check: impl FnMut(&ConfirmationsViewModel) -> Option<R>,
) -> R {
    let subscription = phone.subscribe_confirmations();
    let deadline = tokio::time::Instant::now() + WAIT;
    let mut last = None;
    loop {
        if let Some(found) = check(&phone.confirmations()) {
            return found;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what}：{WAIT:?} 内没有等到，当前：{:#?}",
            last.as_ref().unwrap_or(&phone.confirmations())
        );
        if let Some(view) = tokio::time::timeout_at(deadline, subscription.next())
            .await
            .ok()
            .flatten()
        {
            last = Some(view);
        }
    }
}

fn in_ten_minutes() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 600_000
}

/// 核心的列表里新出一张 Bot 房间发起的卡片：会话横幅的依据有它、房间也对。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_card_for_a_bot_room_lands_in_that_room() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 这些测试只要确认列表（假核心 HTTP），不需要 Matrix：挡掉登录，省掉每台手机 300+/秒的同步空转。
    home.fail_login_tokens(usize::MAX);
    phone.connect(&home);

    home.set_confirmations(
        vec![bot_room_card("install", "!hermes:wonton.localhost")],
        vec![],
    );
    let pending = wait_confirmations(&phone, "卡片在等回答", |view| {
        view.pending.first().cloned()
    })
    .await;
    assert_eq!(pending.card.id, "install");
    assert_eq!(pending.card.state, ConfirmationState::Pending);
    assert_eq!(pending.card.choices.len(), 2);
    // vm 和聊天视图是两个发送端，等聊天视图跟上来再断言，读到半个发布是测试自己的竞态。
    let banner = phone
        .wait_chat("会话横幅的依据里有这张卡片", |view| {
            view.pending_confirmations
                .iter()
                .find(|card| card.id == "install")
                .cloned()
        })
        .await;
    assert_eq!(banner.room_id.as_deref(), Some("!hermes:wonton.localhost"));
    assert_eq!(banner.expires_at_ms, pending.card.expires_at_ms);
    assert!(matches!(
        &banner.requester,
        Some(wonton_client::ConfirmationRequester::Bot { .. })
    ));
}

/// 没有 `origin` 的卡片（核心自己发起的，比如服务器配对）只在列表里，横幅不带房间。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_card_without_origin_stays_out_of_rooms() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 这些测试只要确认列表（假核心 HTTP），不需要 Matrix：挡掉登录，省掉每台手机 300+/秒的同步空转。
    home.fail_login_tokens(usize::MAX);
    phone.connect(&home);

    home.set_confirmations(vec![card("pairing", None, in_ten_minutes())], vec![]);
    wait_confirmations(&phone, "卡片在等回答", |view| {
        view.pending.first().map(|view| view.card.id.clone())
    })
    .await;
    // vm 和聊天视图是两个发送端，等聊天视图跟上来再断言，读到半个发布是测试自己的竞态。
    let room_id = phone
        .wait_chat("等回答的依据里有这张卡片", |view| {
            view.pending_confirmations
                .iter()
                .find(|card| card.id == "pairing")
                .map(|card| card.room_id.clone())
        })
        .await;
    assert_eq!(room_id, None, "没有来源会话的卡片不挂到任何房间");
}

/// 回答之后，列表里同一张先变发送中、再变结果；核心不收（另一台设备答过）时如实显示。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn answering_shows_progress_then_the_result() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 这些测试只要确认列表（假核心 HTTP），不需要 Matrix：挡掉登录，省掉每台手机 300+/秒的同步空转。
    home.fail_login_tokens(usize::MAX);
    phone.connect(&home);

    home.set_confirmations(
        vec![bot_room_card("install", "!hermes:wonton.localhost")],
        vec![],
    );
    wait_confirmations(&phone, "卡片在等回答", |view| {
        (view.pending.len() == 1).then_some(())
    })
    .await;

    // 正在发：按钮还在，但不能点。
    home.hold_answers(true);
    phone
        .dispatch(Action::AnswerConfirmation {
            id: "install".to_owned(),
            answer: Answer::Approve,
            choice_id: None,
            secrets: Default::default(),
            settings: Default::default(),
        })
        .unwrap();
    wait_confirmations(&phone, "正在发", |view| {
        (view.pending[0].sending == Some(Answer::Approve)).then_some(())
    })
    .await;
    home.hold_answers(false);

    // 核心收下了：结果先到（回答接口直接回的），卡片挪进结束的，显示执行中的结果、按钮消失。
    let view = wait_confirmations(&phone, "核心收下回答", |view| {
        view.finished
            .iter()
            .find(|view| view.card.id == "install")
            .filter(|view| view.card.state == ConfirmationState::Running)
            .cloned()
    })
    .await;
    assert_eq!(view.sending, None);
    assert!(view.card.choices.is_empty());
    // 连点不再发一次。
    phone
        .dispatch(Action::AnswerConfirmation {
            id: "install".to_owned(),
            answer: Answer::Approve,
            choice_id: None,
            secrets: Default::default(),
            settings: Default::default(),
        })
        .unwrap();
    assert_eq!(home.confirmation_answers().len(), 1);

    // 核心的列表更新：卡片挪进结束的。
    home.set_confirmations(
        vec![],
        vec![json!({
            "id": "install",
            "state": {"type": "running"},
            "title": "install 要确认",
            "lines": [],
            "choices": [],
            "expires_at_ms": in_ten_minutes(),
            "origin": {"room_id": "!hermes:wonton.localhost"},
        })],
    );
    let (finished, settled) = wait_confirmations(&phone, "列表里换成结束的", |view| {
        (!view.finished.is_empty() && !phone.chat().settled_confirmations.is_empty()).then(|| {
            (
                view.finished[0].card.state.clone(),
                phone.chat().settled_confirmations.clone(),
            )
        })
    })
    .await;
    assert_eq!(finished, ConfirmationState::Running);
    assert_eq!(settled, ["install"]);
}

/// 核心回 409（主人的另一台设备答过了）：如实显示，按钮消失，列进不用回答的。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_refused_answer_shows_why() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 这些测试只要确认列表（假核心 HTTP），不需要 Matrix：挡掉登录，省掉每台手机 300+/秒的同步空转。
    home.fail_login_tokens(usize::MAX);
    phone.connect(&home);

    home.set_confirmations(
        vec![bot_room_card("answered", "!hermes:wonton.localhost")],
        vec![],
    );
    wait_confirmations(&phone, "第二张卡片", |view| {
        view.pending
            .iter()
            .any(|view| view.card.id == "answered")
            .then_some(())
    })
    .await;
    home.reply_to_next_answer(
        409,
        json!({"error": {"kind": "already_answered", "message": "这个确认已经回答过了"}}),
    );
    phone
        .dispatch(Action::AnswerConfirmation {
            id: "answered".to_owned(),
            answer: Answer::Reject,
            choice_id: None,
            secrets: Default::default(),
            settings: Default::default(),
        })
        .unwrap();
    let view = wait_confirmations(&phone, "核心不收", |view| {
        view.finished
            .iter()
            .find(|view| view.card.id == "answered")
            .filter(|view| view.problem.is_some())
            .cloned()
    })
    .await;
    assert_eq!(view.problem.unwrap().reason, ChatFailure::InvalidInput);
    assert!(view.card.choices.is_empty());
    assert_eq!(view.sending, None);
    // vm 和聊天视图是两个发送端（见 answering_shows_progress_then_the_result），等聊天视图跟上来再断言。
    phone
        .wait_chat("卡片列进不用回答的", |view| {
            view.settled_confirmations
                .contains(&"answered".to_owned())
                .then_some(())
        })
        .await;
}

/// 长轮询断了：问题按类别报成传输类，上次的列表保留；重试后恢复。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_broken_poll_keeps_the_last_list_until_it_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 这些测试只要确认列表（假核心 HTTP），不需要 Matrix：挡掉登录，省掉每台手机 300+/秒的同步空转。
    home.fail_login_tokens(usize::MAX);
    phone.connect(&home);

    home.set_confirmations(
        vec![bot_room_card("install", "!hermes:wonton.localhost")],
        vec![],
    );
    wait_confirmations(&phone, "卡片在等回答", |view| {
        (view.pending.len() == 1).then_some(())
    })
    .await;

    // 接下来的一次取列表回 502：本地的代理连不上隧道时也是这个状态，报传输类；列表留着。
    home.fail_confirmations(1);
    wait_confirmations(&phone, "报了传输问题", |view| {
        view.problem
            .as_ref()
            .filter(|problem| problem.reason == ChatFailure::TransportUnavailable)
            .map(|_| ())
    })
    .await;
    let view = phone.confirmations();
    assert_eq!(view.pending.len(), 1, "上次的列表保留");
    assert!(!view.loading, "取到过列表就不是加载中");
    assert_eq!(
        phone.chat().pending_confirmations.len(),
        1,
        "会话横幅的依据也保留"
    );

    // 主人点了重试：立即再取（不等退避），恢复后问题清掉。核心的列表这时有了第二张。
    home.set_confirmations(
        vec![
            bot_room_card("install", "!hermes:wonton.localhost"),
            bot_room_card("uninstall", "!grafana:wonton.localhost"),
        ],
        vec![],
    );
    home.fail_confirmations(0);
    phone.dispatch(Action::RetryConfirmations).unwrap();
    wait_confirmations(&phone, "重试后恢复", |view| {
        (view.problem.is_none() && view.pending.len() == 2).then_some(())
    })
    .await;
    // 重试真的立即取了：502 那次之后又来了一次请求。
    assert!(home.confirmations_requests() >= 3);
}

/// 期限到了核心还没标：这台设备把卡片显示成过期，挪进结束的；核心的列表更新后两边一致。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_card_that_runs_out_is_settled_without_a_core_update() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 这些测试只要确认列表（假核心 HTTP），不需要 Matrix：挡掉登录，省掉每台手机 300+/秒的同步空转。
    home.fail_login_tokens(usize::MAX);
    phone.connect(&home);

    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 1500;
    home.set_confirmations(
        vec![card("flaky", Some("!hermes:wonton.localhost"), expires)],
        vec![],
    );
    wait_confirmations(&phone, "卡片在等回答", |view| {
        (view.pending.len() == 1).then_some(())
    })
    .await;

    let view = wait_confirmations(&phone, "过期挪进结束的", |view| {
        view.finished
            .iter()
            .find(|view| view.card.id == "flaky")
            .filter(|view| view.card.state == ConfirmationState::Expired)
            .cloned()
    })
    .await;
    assert_eq!(view.card.choices, vec![] as Vec<wonton_client::CardChoice>);
    // vm 和聊天视图是两个发送端（见 answering_shows_progress_then_the_result），等聊天视图跟上来再断言。
    phone
        .wait_chat("过期卡片列进不用回答的", |view| {
            (view.settled_confirmations.contains(&"flaky".to_owned())
                && view.pending_confirmations.is_empty())
            .then_some(())
        })
        .await;

    // 核心的列表更新成过期：还是同一张，不再冒出按钮。
    home.set_confirmations(
        vec![],
        vec![json!({
            "id": "flaky",
            "state": {"type": "expired"},
            "title": "flaky 要确认",
            "lines": [],
            "choices": [],
            "expires_at_ms": expires,
        })],
    );
    wait_confirmations(&phone, "核心标成过期", |view| {
        view.finished
            .iter()
            .find(|view| view.card.id == "flaky")
            .filter(|view| view.card.state == ConfirmationState::Expired)
            .cloned()
    })
    .await;
}

/// 回答发不出去：如实显示原因，按钮还在，可以再点。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_answer_that_does_not_land_says_why_and_stays_clickable() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 这些测试只要确认列表（假核心 HTTP），不需要 Matrix：挡掉登录，省掉每台手机 300+/秒的同步空转。
    home.fail_login_tokens(usize::MAX);
    phone.connect(&home);

    home.set_confirmations(
        vec![bot_room_card("flaky", "!hermes:wonton.localhost")],
        vec![],
    );
    wait_confirmations(&phone, "卡片在等回答", |view| {
        (view.pending.len() == 1).then_some(())
    })
    .await;

    home.reply_to_next_answer(502, json!({}));
    phone
        .dispatch(Action::AnswerConfirmation {
            id: "flaky".to_owned(),
            answer: Answer::Approve,
            choice_id: None,
            secrets: Default::default(),
            settings: Default::default(),
        })
        .unwrap();
    let view = wait_confirmations(&phone, "没答上", |view| {
        view.pending
            .iter()
            .find(|view| view.card.id == "flaky")
            .filter(|view| view.problem.is_some())
            .cloned()
    })
    .await;
    assert_eq!(
        view.problem.unwrap().reason,
        ChatFailure::TransportUnavailable
    );
    assert_eq!(view.card.state, ConfirmationState::Pending);
    assert_eq!(view.card.choices.len(), 2);
    assert_eq!(view.sending, None);

    // 再点一次答上了。
    phone
        .dispatch(Action::AnswerConfirmation {
            id: "flaky".to_owned(),
            answer: Answer::Approve,
            choice_id: None,
            secrets: Default::default(),
            settings: Default::default(),
        })
        .unwrap();
    wait_confirmations(&phone, "再点一次答上了", |view| {
        view.finished
            .iter()
            .find(|view| view.card.id == "flaky")
            .filter(|view| view.card.state == ConfirmationState::Running && view.problem.is_none())
            .cloned()
    })
    .await;
}

/// 不认识的回答发不出去（输入非法），列表不受影响。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_unknown_answer_is_rejected_before_sending() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 这些测试只要确认列表（假核心 HTTP），不需要 Matrix：挡掉登录，省掉每台手机 300+/秒的同步空转。
    home.fail_login_tokens(usize::MAX);
    phone.connect(&home);

    home.set_confirmations(
        vec![bot_room_card("install", "!hermes:wonton.localhost")],
        vec![],
    );
    wait_confirmations(&phone, "卡片在等回答", |view| {
        (view.pending.len() == 1).then_some(())
    })
    .await;
    assert!(matches!(
        phone.dispatch(Action::AnswerConfirmation {
            id: "install".to_owned(),
            answer: Answer::Unknown,
            choice_id: None,
            secrets: Default::default(),
            settings: Default::default(),
        }),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(home.confirmation_answers().is_empty());
    let view = phone.confirmations();
    assert_eq!(view.pending[0].sending, None);
}

/// 放行时段（ADR-0024）：生效中显示“停止”，点了先停住按钮，核心收下后显示已结束的说明。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_owner_can_stop_a_window_and_sees_it_end() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 这些测试只要确认列表（假核心 HTTP），不需要 Matrix：挡掉登录，省掉每台手机 300+/秒的同步空转。
    home.fail_login_tokens(usize::MAX);
    phone.connect(&home);

    let expires = in_ten_minutes() + 600_000;
    let mut card = bot_room_card("windowed", "!hermes:wonton.localhost");
    card["window"] = json!({"expires_at_ms": expires, "allowed": 0, "ended": Value::Null});
    home.set_confirmations(vec![card], vec![]);
    let window = wait_confirmations(&phone, "时段生效中", |view| {
        view.pending
            .first()
            .map(|view| view.window.clone())
            .filter(|window| {
                matches!(
                    window,
                    Some(wonton_client::WindowView::Active {
                        stopping: false,
                        problem: None,
                        ..
                    })
                )
            })
    })
    .await;
    assert_eq!(
        window,
        Some(wonton_client::WindowView::Active {
            expires_at_ms: expires,
            stopping: false,
            problem: None,
        })
    );

    phone
        .dispatch(Action::StopConfirmationWindow {
            id: "windowed".to_owned(),
        })
        .unwrap();
    wait_confirmations(&phone, "停止在发", |view| {
        view.pending
            .first()
            .map(|view| view.window.clone())
            .filter(|window| {
                matches!(
                    window,
                    Some(wonton_client::WindowView::Active { stopping: true, .. })
                )
            })
    })
    .await;
    wait_confirmations(&phone, "停下来了", |view| {
        view.pending
            .first()
            .map(|view| view.window.clone())
            .filter(|window| matches!(window, Some(wonton_client::WindowView::Ended { .. })))
    })
    .await;
    assert_eq!(home.window_stops(), ["windowed"]);
}

/// 通知房间（Wonton 私聊）是只读的：消息发不进去，别的私聊不受影响。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_notice_room_is_read_only_and_other_chats_are_not() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let (_core, notices) = core_room(&phone, &home).await;
    let friend = home.bot("friend", "Friend").await;
    let chat = joined_direct_chat(&phone, &friend).await;
    let read_only = |room: &str| {
        let room = room.to_owned();
        move |view: &wonton_client::ChatViewModel| {
            view.rooms
                .iter()
                .find(|row| row.room_id == room && row.direct_user_id.is_some())
                .map(|row| row.is_read_only)
        }
    };
    assert!(
        phone
            .wait_chat("通知房间只读", read_only(notices.as_str()))
            .await
    );
    assert!(
        !phone
            .wait_chat("别的私聊能发", read_only(chat.as_str()))
            .await
    );

    let send = |room_id: &str| {
        phone.dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: TimelineTarget {
                    room_id: room_id.to_owned(),
                    thread_root: None,
                },
                body: "你好".to_owned(),
                reply_to: None,
            },
        })
    };
    assert!(send(notices.as_str()).is_err());
    assert!(send(chat.as_str()).is_ok());
}
