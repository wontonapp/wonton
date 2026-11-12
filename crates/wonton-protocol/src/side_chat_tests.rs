//! 旁聊三个 Matrix 事件内容的契约样例（ADR-0093 §1、§5）。样例和 wonton-docs 插件规范里的
//! 一致：桥、Hermes 插件发出的事件，和客户端核心读到的，都过这一组。

use serde_json::{from_value, json, to_value};

use crate::side_chat::{SideChatResult, SideChatResultState, SideChatRoot, SideChatSupport};

#[test]
fn root_round_trips() {
    let root = json!({ "v": 1, "title": "整理一下手机里的照片" });
    let parsed: SideChatRoot = from_value(root.clone()).expect("根消息字段读得懂");
    assert_eq!(
        parsed,
        SideChatRoot {
            v: 1,
            title: "整理一下手机里的照片".into()
        }
    );
    assert_eq!(to_value(&parsed).expect("根消息字段写回去"), root);
}

#[test]
fn result_round_trips_all_states() {
    let done = json!({
        "v": 1,
        "root": "$root",
        "title": "整理一下手机里的照片",
        "state": "done",
    });
    let parsed: SideChatResult = from_value(done.clone()).expect("结果行读得懂");
    assert_eq!(
        parsed,
        SideChatResult {
            v: 1,
            root: "$root".into(),
            title: "整理一下手机里的照片".into(),
            state: SideChatResultState::Done,
        }
    );
    assert_eq!(to_value(&parsed).expect("结果行写回去"), done);

    let ended = json!({
        "v": 1,
        "root": "$root",
        "title": "整理一下手机里的照片",
        "state": "ended",
    });
    let parsed: SideChatResult = from_value(ended).expect("ended 读得懂");
    assert_eq!(parsed.state, SideChatResultState::Ended);
}

#[test]
fn result_unknown_state_fails() {
    // state 不认识时反序列化失败：调用方把整条当普通 m.notice 显示，不猜（ADR-0093 §7）。
    let unknown = json!({
        "v": 1,
        "root": "$root",
        "title": "整理一下手机里的照片",
        "state": "archived",
    });
    assert!(from_value::<SideChatResult>(unknown).is_err());
}

#[test]
fn result_missing_fields_fail() {
    let no_root = json!({ "v": 1, "title": "整理一下手机里的照片", "state": "done" });
    assert!(from_value::<SideChatResult>(no_root).is_err());

    let no_title = json!({ "v": 1, "root": "$root", "state": "done" });
    assert!(from_value::<SideChatResult>(no_title).is_err());
}

#[test]
fn root_missing_title_fails() {
    assert!(from_value::<SideChatRoot>(json!({ "v": 1 })).is_err());
}

#[test]
fn support_round_trips() {
    let parsed: SideChatSupport = from_value(json!({ "v": 1 })).expect("支持声明读得懂");
    assert_eq!(parsed, SideChatSupport { v: 1 });
    assert_eq!(
        to_value(&parsed).expect("支持声明写回去"),
        json!({ "v": 1 })
    );
}
