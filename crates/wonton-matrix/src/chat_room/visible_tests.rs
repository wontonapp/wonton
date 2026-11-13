use serde_json::{Map, Value, json};

use super::*;
use crate::{Message, MessageBody, SendStatus};

fn message(id: &str, custom: Value) -> TimelineEntry {
    let Value::Object(custom) = custom else {
        panic!("自定义字段要是对象");
    };
    TimelineEntry {
        id: id.to_owned(),
        kind: EntryKind::Message(Box::new(Message {
            event_id: Some(format!("${id}").parse().unwrap()),
            sender: "@bot:s".parse().unwrap(),
            sender_name: "Bot".to_owned(),
            is_own: false,
            timestamp_ms: 0,
            status: SendStatus::Sent,
            body: MessageBody::Notice(id.to_owned()),
            custom,
            mentions: Vec::new(),
            thread_root: None,
            thread_replies: None,
            edited: false,
            read_by: Vec::new(),
            reply: None,
            group: None,
            agent_run: None,
            can_redact: false,
        })),
    }
}

fn result(id: &str, root: &str) -> TimelineEntry {
    message(
        id,
        json!({"app.wonton.side_chat_result": {"v": 1, "root": root, "title": "t", "state": "done"}}),
    )
}

fn plain(id: &str) -> TimelineEntry {
    message(id, Value::Object(Map::new()))
}

fn ids(entries: &Vector<TimelineEntry>) -> Vec<String> {
    entries.iter().map(|entry| entry.id.clone()).collect()
}

/// 调用方按换算出来的增量维护的列表，每批之后都和应显示的一样。
struct Shell {
    visible: Visible,
    shell: Vector<TimelineEntry>,
}

impl Shell {
    fn new(focus: Focus, entries: Vec<TimelineEntry>) -> Self {
        let mut visible = Visible {
            focus,
            full: entries.into_iter().collect(),
            shown: Vec::new(),
        };
        visible.shown = visible.wanted();
        let shell = visible.visible().into_iter().collect();
        Self { visible, shell }
    }

    fn apply(&mut self, batch: Vec<VectorDiff<TimelineEntry>>) {
        for diff in self.visible.apply(batch) {
            diff.apply(&mut self.shell);
        }
        let expected: Vector<TimelineEntry> = self.visible.visible().into_iter().collect();
        assert_eq!(ids(&self.shell), ids(&expected));
    }
}

/// 行为：主对话里同一个旁聊只显示最新一条结果行；新的来了旧的藏起来，新的删掉旧的再露出来；别的旁聊不受影响。
#[test]
fn only_the_latest_result_line_per_side_chat_shows() {
    let mut shell = Shell::new(
        Focus::Main,
        vec![plain("a"), result("r1", "$x"), result("y1", "$y")],
    );
    assert_eq!(ids(&shell.shell), ["a", "r1", "y1"]);

    shell.apply(vec![VectorDiff::PushBack {
        value: result("r2", "$x"),
    }]);
    assert_eq!(ids(&shell.shell), ["a", "y1", "r2"]);

    shell.apply(vec![
        VectorDiff::PushBack { value: plain("b") },
        VectorDiff::Insert {
            index: 0,
            value: plain("old"),
        },
    ]);
    assert_eq!(ids(&shell.shell), ["old", "a", "y1", "r2", "b"]);

    // 最新一条被撤回成别的东西：上一条重新露出来。
    shell.apply(vec![VectorDiff::Set {
        index: 4,
        value: plain("r2"),
    }]);
    assert_eq!(ids(&shell.shell), ["old", "a", "r1", "y1", "r2", "b"]);

    shell.apply(vec![VectorDiff::Remove { index: 2 }, VectorDiff::PopFront]);
    assert_eq!(ids(&shell.shell), ["a", "y1", "r2", "b"]);

    shell.apply(vec![VectorDiff::Truncate { length: 1 }]);
    assert_eq!(ids(&shell.shell), ["a"]);
}

/// 行为：旁聊时间线里带标记的根不作为条目，话题里的回复照常显示；不带标记的根照常是第一条。
#[test]
fn a_marked_root_is_not_an_item_in_its_side_chat() {
    let marked = message(
        "root",
        json!({"app.wonton.side_chat": {"v": 1, "title": "整理照片"}}),
    );
    let mut shell = Shell::new(Focus::Thread("$root".parse().unwrap()), vec![marked]);
    assert!(shell.shell.is_empty());
    shell.apply(vec![VectorDiff::PushBack {
        value: plain("reply"),
    }]);
    assert_eq!(ids(&shell.shell), ["reply"]);

    let unmarked = Shell::new(Focus::Thread("$root".parse().unwrap()), vec![plain("root")]);
    assert_eq!(ids(&unmarked.shell), ["root"]);

    // 平铺的时间线（桥用的）什么都不藏。
    let flat = Shell::new(Focus::Flat, vec![result("r1", "$x"), result("r2", "$x")]);
    assert_eq!(ids(&flat.shell), ["r1", "r2"]);
}
