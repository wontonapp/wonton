//! 顶栏的“正在输入”。

use super::*;

/// 行为：谁发了消息就把谁从名单里去掉，其他人和先后顺序不动；都不在名单里时不算变化。
#[test]
fn a_sender_leaves_the_list_alone() {
    let typing = Typing::seeded(&[("@a:home.example", "A"), ("@b:home.example", "B")]);
    assert!(typing.forget(&["@a:home.example".into()]));
    assert_eq!(typing.names(), ["B"]);
    assert!(!typing.forget(&["@a:home.example".into()]));
    assert!(!typing.forget(&[]));
    assert_eq!(typing.names(), ["B"]);
}

/// 行为：名单没变不重复发，变了才发。
#[test]
fn only_a_changed_list_is_a_change() {
    let typing = Typing::seeded(&[("@a:home.example", "A")]);
    assert!(typing.take_changed());
    assert!(!typing.take_changed());
    typing.forget(&["@a:home.example".into()]);
    assert!(typing.take_changed());
    assert!(!typing.take_changed());
}
