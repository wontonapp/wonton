use serde_json::{Value, json};

use super::*;

fn event(id: &str, sender: &str, ts: u64, content: Value) -> RawEvent {
    RawEvent::parse(
        &json!({
            "type": "m.room.message",
            "event_id": id,
            "sender": sender,
            "origin_server_ts": ts,
            "content": content,
        })
        .to_string(),
    )
    .expect("消息事件读得懂")
}

fn in_thread(root: &str, body: &str) -> Value {
    json!({
        "msgtype": "m.text",
        "body": body,
        "m.relates_to": {"rel_type": "m.thread", "event_id": root},
    })
}

fn result(root: &str, state: &str) -> Value {
    json!({
        "msgtype": "m.notice",
        "body": "旁聊「照片」：已完成",
        "m.relates_to": {"m.in_reply_to": {"event_id": root}},
        "app.wonton.side_chat_result": {"v": 1, "root": root, "title": "照片", "state": state},
    })
}

fn status(state: &str) -> Value {
    json!({
        "msgtype": "m.notice",
        "body": "在干活",
        "m.agent": {"v": 1, "state": state},
    })
}

#[test]
fn the_title_is_the_first_line_cut_to_twenty_characters() {
    assert_eq!(
        title_from_body("整理一下手机里的照片\n顺便删掉重复的"),
        "整理一下手机里的照片"
    );
    assert_eq!(
        title_from_body("  一二三四五六七八九十一二三四五六七八九十多出来的"),
        "一二三四五六七八九十一二三四五六七八九十"
    );
    assert_eq!(title_from_body(""), "");
}

#[test]
fn only_version_one_result_lines_with_all_fields_count() {
    let custom = |field: Value| {
        let mut map = Map::new();
        map.insert(SIDE_CHAT_RESULT_FIELD.to_owned(), field);
        map
    };
    let done = result_line(
        &custom(json!({"v": 1, "root": "$r", "title": "t", "state": "done"})),
        5,
    )
    .expect("完整的结果行");
    assert_eq!(done.state, Some(SideChatResultState::Done));
    assert_eq!(done.timestamp_ms, 5);
    let unknown = result_line(
        &custom(json!({"v": 1, "root": "$r", "title": "t", "state": "archived"})),
        5,
    )
    .expect("不认识的状态也是结果行");
    assert_eq!(unknown.state, None);
    for bad in [
        json!({"v": 2, "root": "$r", "title": "t", "state": "done"}),
        json!({"v": 1, "title": "t", "state": "done"}),
        json!({"v": 1, "root": "$r", "state": "done"}),
        json!({"v": 1, "root": "不是事件", "title": "t", "state": "done"}),
    ] {
        assert_eq!(result_line(&custom(bad.clone()), 5), None, "{bad}");
    }
}

/// 行为：同一个旁聊只留最新一条结果行；带标记的根用标记里的标题，别的客户端开的话题按根的正文取标题。
#[test]
fn facts_keep_the_latest_result_and_titles() {
    let events = [
        event(
            "$marked",
            "@owner:s",
            1,
            json!({"msgtype": "m.notice", "body": "整理照片",
                   "app.wonton.side_chat": {"v": 1, "title": "整理照片"}}),
        ),
        event(
            "$plain",
            "@owner:s",
            2,
            json!({"msgtype": "m.text", "body": "别的客户端开的话题\n第二行"}),
        ),
        event("$r1", "@bot:s", 3, in_thread("$plain", "回复")),
        event("$res1", "@bot:s", 4, result("$marked", "failed")),
        event("$res2", "@bot:s", 5, result("$marked", "done")),
    ];
    let (facts, roots) = facts_of(&events);
    assert_eq!(
        facts.results[&"$marked".parse::<OwnedEventId>().unwrap()].state,
        Some(SideChatResultState::Done)
    );
    assert_eq!(roots.len(), 2);
    assert_eq!(
        facts.titles[&"$marked".parse::<OwnedEventId>().unwrap()],
        "整理照片"
    );
    assert_eq!(
        facts.titles[&"$plain".parse::<OwnedEventId>().unwrap()],
        "别的客户端开的话题"
    );
}

/// 行为：状态消息不算对方的最后一条；编辑把状态改成完成后不再算在跑。
#[test]
fn activity_skips_status_messages_and_follows_their_edits() {
    let own: OwnedUserId = "@owner:s".parse().unwrap();
    let mut events = vec![
        event("$q", "@owner:s", 10, in_thread("$root", "问")),
        event("$a", "@bot:s", 20, in_thread("$root", "答")),
        event("$s", "@bot:s", 30, status("working")),
    ];
    let running = activity_of(&events, &own);
    assert_eq!(running.last_reply_ms, Some(20));
    assert!(running.running);

    events.push(event(
        "$edit",
        "@bot:s",
        40,
        json!({
            "msgtype": "m.notice",
            "body": "* 完成",
            "m.new_content": {"msgtype": "m.notice", "body": "完成", "m.agent": {"v": 1, "state": "done"}},
            "m.relates_to": {"rel_type": "m.replace", "event_id": "$s"},
        }),
    ));
    let finished = activity_of(&events, &own);
    assert!(!finished.running);
    assert_eq!(finished.last_reply_ms, Some(20));
}
