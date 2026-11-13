//! 标题规则（ADR-0093 §1）的单元测试；结果行的事件内容由桥的行为测试断言。

use super::*;

#[test]
fn title_prefers_the_root_marker() {
    let mut titles = Titles::default();
    titles.observe("$sc", "根上的标题".into());
    assert_eq!(title_of(&titles, "$sc", "正文里的标题"), "根上的标题");
}

#[test]
fn title_falls_back_to_the_first_line_of_the_root_body() {
    let titles = Titles::default();
    assert_eq!(title_of(&titles, "$sc", "第一行\n第二行"), "第一行");
    let long = "这是一个超过二十个字符的话题标题，后面的部分不算进标题";
    let expected: String = long.chars().take(20).collect();
    assert_eq!(title_of(&titles, "$sc", long), expected);
}
