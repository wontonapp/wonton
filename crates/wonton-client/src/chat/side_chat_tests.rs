use serde_json::{Map, json};
use wonton_matrix::{MessageBody, SendStatus};

use super::*;

const MINUTE: u64 = 60_000;

fn line(state: Option<SideChatResultState>, timestamp_ms: u64) -> ResultLine {
    ResultLine {
        root: "$root".parse().unwrap(),
        title: "整理照片".to_owned(),
        state,
        timestamp_ms,
    }
}

fn activity(last_reply_ms: Option<u64>, running: bool) -> ThreadActivity {
    ThreadActivity {
        unread: 0,
        replies: 1,
        last_reply_ms,
        running,
    }
}

/// 行为：Bot 最后一条消息之后两分钟没等到结果行、也没有在跑的状态，是“没收到结果”；不到两分钟还是开着。
#[test]
fn a_missing_result_shows_after_two_minutes() {
    let quiet = activity(Some(10 * MINUTE), false);
    assert_eq!(
        card_state(true, None, Some(&quiet), 11 * MINUTE),
        SideChatState::Open
    );
    assert_eq!(
        card_state(true, None, Some(&quiet), 12 * MINUTE),
        SideChatState::ResultMissing
    );
    // 还在干活（ADR-0082 的工作中、等待中）不算没收到。
    let running = activity(Some(10 * MINUTE), true);
    assert_eq!(
        card_state(true, None, Some(&running), 30 * MINUTE),
        SideChatState::Open
    );
    // 还没有 Bot 的消息：开着。
    assert_eq!(
        card_state(true, None, None, 30 * MINUTE),
        SideChatState::Open
    );
}

/// 行为：最新的结果行比 Bot 最后一条消息新就按结果行；结果行之后 Bot 又说了话，过了期限仍没新结果是“没收到结果”。
#[test]
fn the_latest_result_line_wins_until_the_bot_talks_again() {
    let done = line(Some(SideChatResultState::Done), 11 * MINUTE);
    let before = activity(Some(10 * MINUTE), false);
    assert_eq!(
        card_state(true, Some(&done), Some(&before), 60 * MINUTE),
        SideChatState::Done
    );
    let failed = line(Some(SideChatResultState::Failed), 11 * MINUTE);
    assert_eq!(
        card_state(true, Some(&failed), None, 0),
        SideChatState::Failed
    );

    let after = activity(Some(20 * MINUTE), false);
    assert_eq!(
        card_state(true, Some(&done), Some(&after), 21 * MINUTE),
        SideChatState::Done
    );
    assert_eq!(
        card_state(true, Some(&done), Some(&after), 22 * MINUTE),
        SideChatState::ResultMissing
    );
    // 读不懂的结果状态：其他。
    assert_eq!(
        card_state(true, Some(&line(None, 11 * MINUTE)), None, 0),
        SideChatState::Other
    );
}

/// 行为：没有可信的支持声明时不推断“没收到结果”：有结果行按它，没有是“不跟踪”。
#[test]
fn without_a_declaration_the_card_is_untracked() {
    let quiet = activity(Some(10 * MINUTE), false);
    assert_eq!(
        card_state(false, None, Some(&quiet), 60 * MINUTE),
        SideChatState::Untracked
    );
    let ended = line(Some(SideChatResultState::Ended), 5 * MINUTE);
    assert_eq!(
        card_state(false, Some(&ended), Some(&quiet), 60 * MINUTE),
        SideChatState::Ended
    );
}

fn declaration(sender: &str, state_key: &str, v: u64) -> Value {
    json!({
        "type": SIDE_CHAT_SUPPORT_EVENT,
        "sender": sender,
        "state_key": state_key,
        "content": {"v": v},
    })
}

fn member(user: &str, membership: &str) -> Value {
    json!({
        "type": "m.room.member",
        "sender": user,
        "state_key": user,
        "content": {"membership": membership},
    })
}

/// 行为：只认自己给自己发的、版本是 1 的、还在房间里的成员的声明。
#[test]
fn only_trusted_declarations_count() {
    let state = vec![
        member("@bot:s", "join"),
        member("@left:s", "leave"),
        member("@owner:s", "join"),
        declaration("@bot:s", "@bot:s", 1),
        // 替别人声明的不算。
        declaration("@owner:s", "@bot:s", 1),
        // 已经离开的不算。
        declaration("@left:s", "@left:s", 1),
        // 读不懂的版本不算。
        declaration("@owner:s", "@owner:s", 2),
    ];
    let users: Vec<String> = declarers(&state).iter().map(ToString::to_string).collect();
    assert_eq!(users, ["@bot:s"]);
}

/// 行为：支持声明还要声明者是认得的 Bot；Bot 列表晚到时按新的列表算。
#[test]
fn support_needs_a_known_bot() {
    let support = Support::default();
    let room: OwnedRoomId = "!r:s".parse().unwrap();
    lock(&support.declared).insert(room.clone(), vec!["@bot:s".parse().unwrap()]);
    assert!(!support.supports(&room, &HashSet::new()));
    assert!(support.supports(&room, &HashSet::from(["@bot:s".to_owned()])));
    assert!(!support.supports(
        &"!other:s".parse().unwrap(),
        &HashSet::from(["@bot:s".to_owned()])
    ));
}

fn message(id: &str, body: &str, custom: Value, replies: Option<u64>) -> Message {
    let Value::Object(custom) = custom else {
        panic!("自定义字段要是对象");
    };
    Message {
        event_id: Some(id.parse().unwrap()),
        sender: "@bot:s".parse().unwrap(),
        sender_name: "Bot".to_owned(),
        is_own: false,
        timestamp_ms: 0,
        status: SendStatus::Sent,
        body: MessageBody::Notice(body.to_owned()),
        custom,
        mentions: Vec::new(),
        thread_root: None,
        thread_replies: replies,
        edited: false,
        read_by: Vec::new(),
        reply: None,
        group: None,
        agent_run: None,
        can_redact: false,
    }
}

fn facts() -> SideChatFacts {
    let root: OwnedEventId = "$root".parse().unwrap();
    let mut facts = SideChatFacts::default();
    facts.titles.insert(root.clone(), "整理照片".to_owned());
    facts.threads.insert(
        root,
        ThreadActivity {
            unread: 2,
            replies: 3,
            last_reply_ms: Some(10 * MINUTE),
            running: false,
        },
    );
    facts
}

/// 行为：主对话里带标记的根显示成旁聊卡（标题、回复数、未读、状态），结果行显示成结果条目，有回复的普通消息
/// 带卡片；旁聊时间线里都照普通消息显示。
#[test]
fn the_main_timeline_shows_cards_and_result_lines() {
    let view = SideChatsView::of(None, &facts(), true, 11 * MINUTE);
    let root = message(
        "$root",
        "整理照片",
        json!({"app.wonton.side_chat": {"v": 1, "title": "整理照片"}}),
        Some(3),
    );
    let Some(TimelineContent::SideChat { card }) = view.content(&root) else {
        panic!("带标记的根应是旁聊卡");
    };
    assert_eq!(
        (
            card.title.as_str(),
            card.reply_count,
            card.unread_count,
            card.state
        ),
        ("整理照片", 3, 2, SideChatState::Open)
    );

    let result = message(
        "$result",
        "照片整理好了",
        json!({"app.wonton.side_chat_result": {"v": 1, "root": "$root", "title": "整理照片", "state": "done"}}),
        None,
    );
    assert!(matches!(
        view.content(&result),
        Some(TimelineContent::SideChatResult {
            state: SideChatState::Done,
            ..
        })
    ));

    let plain = message("$plain", "第一行\n第二行", json!({}), Some(1));
    assert!(view.content(&plain).is_none());
    let card = view.card_of(&plain).expect("有回复的消息带卡片");
    assert_eq!((card.title.as_str(), card.reply_count), ("第一行", 1));
    assert!(
        view.card_of(&message("$none", "x", json!({}), Some(0)))
            .is_none()
    );

    let thread = SideChatsView::of(Some("$root".parse().unwrap()), &facts(), true, 11 * MINUTE);
    assert!(thread.content(&root).is_none());
    assert!(thread.content(&result).is_none());
    assert!(thread.card_of(&plain).is_none());
    let header = thread.header().expect("旁聊时间线有头部");
    assert_eq!(
        (header.title.as_str(), header.state),
        ("整理照片", SideChatState::Open)
    );
    assert!(view.header().is_none());
}

/// 行为：结果行的状态读不懂时照普通消息显示（新版 Bot 写的状态，旧 App 不认得）。
#[test]
fn an_unknown_result_state_is_a_plain_message() {
    let view = SideChatsView::of(None, &SideChatFacts::default(), true, 0);
    let custom: Map<String, Value> = Map::new();
    let unknown = message(
        "$result",
        "?",
        json!({"app.wonton.side_chat_result": {"v": 1, "root": "$root", "title": "t", "state": "paused"}}),
        None,
    );
    assert!(view.content(&unknown).is_none());
    assert!(
        view.content(&message("$m", "x", Value::Object(custom), None))
            .is_none()
    );
}
