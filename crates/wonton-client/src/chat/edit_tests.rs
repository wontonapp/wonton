use wonton_matrix::Draft;

use super::{Edit, summary};

fn saved() -> Draft {
    Draft {
        body: "之前的草稿".to_owned(),
        reply_to: None,
    }
}

#[test]
fn the_summary_is_the_first_non_empty_line_and_is_truncated() {
    assert_eq!(summary("  \n  第一行  \n第二行"), "第一行");
    assert_eq!(summary("\n\n"), "");
    let long = "字".repeat(100);
    assert!(
        summary(&long).chars().count() < long.chars().count(),
        "长正文要截短"
    );
}

#[test]
fn a_late_begin_after_a_newer_request_does_not_apply() {
    let edit = Edit::default();
    let first = edit.begin_token();
    let second = edit.begin_token();
    assert!(edit.begin(second, "i2".into(), "$b".into(), "新的", saved(), |_| {}));
    assert!(!edit.begin(first, "i1".into(), "$a".into(), "旧的", saved(), |_| {}));
    assert_eq!(edit.card().unwrap().event_id, "$b");
}

#[test]
fn cancelling_gives_the_saved_draft_back_and_clears_the_state() {
    let edit = Edit::default();
    let token = edit.begin_token();
    assert!(edit.begin(token, "i".into(), "$a".into(), "改成这样", saved(), |_| {}));
    let (event_id, editing, draft) = edit.take().expect("在编辑");
    assert_eq!(event_id, "$a");
    // 也交出放进框里的原文：退出时输入框里还是它，才拿原草稿盖回去。
    assert_eq!(editing, "改成这样");
    assert_eq!(draft.body, "之前的草稿");
    assert!(edit.card().is_none());
    // 取过之后原样的编辑态不再补发一批。
    assert!(edit.take_changed());
    assert!(!edit.take_changed());
}

#[test]
fn the_change_flag_only_fires_when_the_state_really_moved() {
    let edit = Edit::default();
    assert!(edit.take_changed()); // 第一次发出去时带上“不在编辑”
    assert!(!edit.take_changed());
    let token = edit.begin_token();
    assert!(edit.begin(token, "i".into(), "$a".into(), "正文", saved(), |_| {}));
    assert!(edit.take_changed());
    assert!(!edit.take_changed());
}

#[test]
fn switching_messages_keeps_the_first_saved_draft() {
    let edit = Edit::default();
    let first = edit.begin_token();
    assert!(edit.begin(first, "i1".into(), "$a".into(), "第一条", saved(), |_| {}));
    let second = edit.begin_token();
    assert!(edit.begin(
        second,
        "i2".into(),
        "$b".into(),
        "第二条",
        edit.saved().expect("还留着最初那份"),
        |_| {},
    ));
    let (event_id, editing, draft) = edit.take().expect("在编辑");
    assert_eq!(event_id, "$b");
    // 换过目标，交出的是换过去时框里那句。
    assert_eq!(editing, "第二条");
    assert_eq!(draft.body, "之前的草稿");
}
