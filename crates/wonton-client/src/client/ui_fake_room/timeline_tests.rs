//! 第一批的落点和最后一条：停在别处，最后一条是高的待发消息。

use super::super::layout::LAST_LINE;
use super::*;
use wonton_protocol::{MessageContent, SendState, TimelineContent, TimelineDiff};

#[test]
fn timeline_lands_away_from_a_tall_last_message() {
    let update = timeline("!ui-fake:home.example");
    let [TimelineDiff::Reset { items }] = update.diffs.as_slice() else {
        panic!("the first batch must be a reset");
    };
    assert_eq!(items.len(), ITEMS + 3);
    assert_ne!(
        update.anchor.as_deref(),
        items.last().map(|item| item.id.as_str())
    );
    let Some(TimelineContent::Message { message }) = items.last().map(|item| &item.content) else {
        panic!("the last item must be a message");
    };
    let MessageContent::Text { blocks, .. } = &message.body else {
        panic!("the last item must be text");
    };
    assert_eq!(blocks.last().unwrap().spans[0].text, LAST_LINE);
    // 最后一条是待发消息：界面测试靠它验“已保存，等待送达”的脚注和时钟。
    assert_eq!(message.send_state, SendState::Sending { progress: None });
    assert_eq!(message.status, None);
}
