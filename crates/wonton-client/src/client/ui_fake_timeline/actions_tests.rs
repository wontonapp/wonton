//! 时间线动作的分发：跳转与返回、选项按钮、转发和发给没打开的房间里的话。

use std::time::Duration;

use super::super::test_support::*;
use super::*;
use wonton_protocol::{ChatAction, ChoicesState};

/// 界面测试约定的那几步：点引用跳过去、“向下”回来、回到底部清栈、点选项、发“/help”。
#[tokio::test]
async fn the_fake_room_answers_the_agreed_actions() {
    let fakes = FakeTimelines::default();
    let subscription = fakes.subscribe(&target());
    let first = next(&subscription).await;
    let [TimelineDiff::Reset { items }] = first.diffs.as_slice() else {
        panic!("第一批是整条列表");
    };
    let mut items = items.clone();
    assert_eq!(first.typing, ["Echo"]);
    assert_eq!(first.commands[0].text, "/help");
    assert_eq!(
        message(&items, "ui-fake-21").status,
        Some(MessageStatus::Read)
    );
    assert_eq!(
        message(&items, "ui-fake-23").status,
        Some(MessageStatus::Sent)
    );
    assert_eq!(
        message(&items, "ui-fake-1").status,
        Some(MessageStatus::Read)
    );
    assert_eq!(message(&items, "ui-fake-2").status, None);

    fakes.timeline_action(TimelineAction::JumpToMessage {
        target: target(),
        event_id: "$ui-fake-5".into(),
        from_item_id: "ui-fake-21".into(),
    });
    let jumped = next(&subscription).await;
    assert_eq!(
        jumped.focus,
        Some(TimelineFocus {
            item_id: "ui-fake-5".into(),
            highlight: true
        })
    );
    assert_eq!(jumped.back_to.as_deref(), Some("ui-fake-21"));

    fakes.timeline_action(TimelineAction::JumpBack { target: target() });
    let back = next(&subscription).await;
    assert_eq!(
        back.focus,
        Some(TimelineFocus {
            item_id: "ui-fake-21".into(),
            highlight: false
        })
    );
    assert_eq!(back.back_to, None);
    fakes.timeline_action(TimelineAction::JumpBack { target: target() });
    assert!(next(&subscription).await.scroll_to_bottom);

    fakes.timeline_action(TimelineAction::JumpToMessage {
        target: target(),
        event_id: "$ui-fake-5".into(),
        from_item_id: "ui-fake-21".into(),
    });
    next(&subscription).await;
    subscription.set_scroll(TimelineScroll::Bottom);
    assert_eq!(next(&subscription).await.back_to, None);

    fakes.chat_action(&ChatAction::AnswerAgentChoice {
        room_id: ROOM.into(),
        item_id: "ui-fake-22".into(),
        key: "✅".into(),
    });
    for diff in next(&subscription).await.diffs {
        crate::chat::apply_diff(&mut items, diff);
    }
    let choices = message(&items, "ui-fake-22").choices.unwrap();
    assert_eq!(choices.state, ChoicesState::Answered);
    assert!(choices.options[0].chosen && !choices.options[1].chosen);

    fakes.timeline_action(TimelineAction::SendText {
        target: target(),
        body: "/help".into(),
        reply_to: None,
        mentions: vec![],
    });
    let sent = next(&subscription).await;
    assert!(sent.scroll_to_bottom);
    for diff in sent.diffs {
        crate::chat::apply_diff(&mut items, diff);
    }
    assert_eq!(items.len(), ITEMS + 4);
    let last = message(&items, &items.last().unwrap().id.clone());
    assert!(last.is_own);
    assert_eq!(last.run, MessageRun::Last);
    // 24、25、26 和后面刚发出去的 27 都是自己发的，同属一组；26 夹在 25 和 27 之间。
    assert_eq!(message(&items, "ui-fake-26").run, MessageRun::Middle);
}

/// 转发一条老消息到没开的房间：等那个房间打开时补进第一批。老消息在第一千里，不在 `items` 里。
#[tokio::test]
async fn forwarding_reaches_an_unopened_room() {
    let fakes = FakeTimelines::default();
    let echo = fakes.subscribe(&target());
    let _first = next(&echo).await;
    assert_eq!(
        fakes
            .get(&target())
            .expect("Echo 开着")
            .text_of("ui-fake-11")
            .as_deref(),
        Some("Fixture message 11")
    );
    fakes.timeline_action(TimelineAction::ForwardMessage {
        target: target(),
        item_id: "ui-fake-11".into(),
        to_room_id: "!ui-fake-call:home.example".into(),
        comment: None,
    });
    let far = fakes.subscribe(&TimelineTarget {
        room_id: "!ui-fake-call:home.example".into(),
        thread_root: None,
    });
    let first = next(&far).await;
    let mut items = Vec::new();
    for diff in first.diffs {
        crate::chat::apply_diff(&mut items, diff);
    }
    assert_eq!(items.len(), ITEMS + 4);
    let forwarded = message(&items, "ui-fake-forwarded-1");
    let MessageContent::Text { body, .. } = &forwarded.body else {
        panic!("转发的是文字");
    };
    assert_eq!(body, "Fixture message 11");
}
/// 发给没开的房间的一句话：等它打开时补进第一批，排在跟着转发的那条前面。
#[tokio::test]
async fn sent_text_reaches_an_unopened_room() {
    let fakes = FakeTimelines::default();
    let echo = fakes.subscribe(&target());
    let _first = next(&echo).await;
    let far_id = "!ui-fake-call:home.example";
    let far_target = TimelineTarget {
        room_id: far_id.into(),
        thread_root: None,
    };
    for body in ["先说一句", "再转一条"] {
        fakes.timeline_action(TimelineAction::SendText {
            target: far_target.clone(),
            body: body.into(),
            reply_to: None,
            mentions: vec![],
        });
        fakes.timeline_action(TimelineAction::ForwardMessage {
            target: target(),
            item_id: "ui-fake-11".into(),
            to_room_id: far_id.into(),
            comment: None,
        });
    }
    let far = fakes.subscribe(&TimelineTarget {
        room_id: far_id.into(),
        thread_root: None,
    });
    let first = next(&far).await;
    let mut items = Vec::new();
    for diff in first.diffs {
        crate::chat::apply_diff(&mut items, diff);
    }
    assert_eq!(items.len(), ITEMS + 7);
    let bodies: Vec<String> = items[ITEMS + 3..]
        .iter()
        .map(|item| {
            let TimelineContent::Message { message } = &item.content else {
                panic!("补进来的都是消息");
            };
            let MessageContent::Text { body, .. } = &message.body else {
                panic!("补进来的都是文字");
            };
            body.clone()
        })
        .collect();
    assert_eq!(
        bodies,
        [
            "先说一句",
            "Fixture message 11",
            "再转一条",
            "Fixture message 11"
        ]
    );
}

/// 转发带附言：附言去掉首尾空白后排进转发的副本前面，只含空白的附言不落；目标房间没开和先开着都一样。
#[tokio::test]
async fn forward_comment_lands_before_the_copy_and_blank_is_dropped() {
    let far_id = "!ui-fake-call:home.example";
    let mut landed = Vec::new();
    for opened_first in [false, true] {
        let fakes = FakeTimelines::default();
        let echo = fakes.subscribe(&target());
        let _first = next(&echo).await;
        let far_target = TimelineTarget {
            room_id: far_id.into(),
            thread_root: None,
        };
        let opened = opened_first.then(|| fakes.subscribe(&far_target));
        if let Some(far) = &opened {
            let _first = next(far).await;
        }
        for comment in [
            Some("看这条".into()),
            Some("  首尾空白  ".into()),
            Some("  \n".into()),
        ] {
            fakes.timeline_action(TimelineAction::ForwardMessage {
                target: target(),
                item_id: "ui-fake-11".into(),
                to_room_id: far_id.into(),
                comment,
            });
        }
        let far = opened.unwrap_or_else(|| fakes.subscribe(&far_target));
        let mut bodies = Vec::new();
        // 一条附言一条、一份副本一条；到齐就停，多出来的第六条由下面两条断言报出来。整段丢掉时 5 秒就失败，不挂在作业的 45 分钟超时上。
        tokio::time::timeout(Duration::from_secs(5), async {
            while bodies.len() < 5 {
                for diff in next(&far).await.diffs {
                    let TimelineDiff::PushBack { item } = diff else {
                        continue;
                    };
                    let TimelineContent::Message { message } = item.content else {
                        panic!("补进来的都是消息");
                    };
                    let MessageContent::Text { body, .. } = message.body else {
                        panic!("补进来的都是文字");
                    };
                    bodies.push(body);
                }
            }
        })
        .await
        .expect("附言和副本没到齐（等 5 秒）");
        assert!(
            tokio::time::timeout(Duration::from_millis(200), next(&far))
                .await
                .is_err(),
            "收齐五条后还有第六条（空白附言落了）：{bodies:?}"
        );
        landed.push((opened_first, bodies));
    }
    for (opened_first, bodies) in landed {
        assert_eq!(
            bodies,
            [
                "看这条",
                "Fixture message 11",
                "首尾空白",
                "Fixture message 11",
                "Fixture message 11"
            ],
            "目标房间先开着：{opened_first}"
        );
    }
}
