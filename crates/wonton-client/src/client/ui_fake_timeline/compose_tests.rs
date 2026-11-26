//! 输入框与回显：编辑、取消、确认，发送前先收“正在输入”，以及旁聊是另一份时间线。

use super::super::test_support::*;
use super::*;

/// 发送（附件区整单）前先收“正在输入”，主对话和旁聊都按各自的目标收；被拒的发送也照样先收。
#[tokio::test]
async fn send_composer_stops_typing_first() {
    let dir = tempfile::tempdir().unwrap();
    let core =
        crate::client::ClientCore::new_ui_fake(dir.path().to_string_lossy().into_owned()).unwrap();
    let side = TimelineTarget {
        room_id: ROOM.into(),
        thread_root: Some("$ui-fake-5".into()),
    };
    for target in [target(), side] {
        core.dispatch(Action::Timeline {
            action: TimelineAction::SendComposer {
                target: target.clone(),
                body: "Fixture".into(),
                reply_to: None,
                mentions: Vec::new(),
            },
        })
        .unwrap();
    }
    let stops = core.scope().chat.typing_stops_for_test();
    assert_eq!(stops.len(), 2);
    assert_eq!(stops[0].room_id.as_str(), ROOM);
    assert_eq!(stops[0].thread_root, None);
    assert_eq!(
        stops[1].thread_root.as_ref().map(|id| id.as_str()),
        Some("$ui-fake-5")
    );
}

/// 编辑的一整套：菜单只给自己的文字、进编辑态换成原文并存下草稿、取消恢复、确认换正文并标已编辑。
#[tokio::test]
async fn editing_saves_and_restores_the_draft() {
    let fakes = FakeTimelines::default();
    let subscription = fakes.subscribe(&target());
    let batch = next(&subscription).await;
    let [TimelineDiff::Reset { items }] = batch.diffs.as_slice() else {
        panic!("第一批是整条时间线");
    };
    assert!(
        message(items, "ui-fake-15")
            .actions
            .iter()
            .any(|action| matches!(action, MessageAction::Edit { .. })),
        "自己的消息菜单里没有编辑"
    );
    assert!(
        !message(items, "ui-fake-14")
            .actions
            .iter()
            .any(|action| matches!(action, MessageAction::Edit { .. })),
        "别人的消息菜单里有编辑"
    );

    fakes.timeline_action(TimelineAction::SetDraft {
        target: target(),
        body: "draft".into(),
        reply_to: None,
    });
    fakes.timeline_action(TimelineAction::StartEdit {
        target: target(),
        item_id: "ui-fake-15".into(),
    });
    let editing = next(&subscription).await.editing.expect("进了编辑态");
    assert_eq!(editing.item_id, "ui-fake-15");
    assert_eq!(editing.summary, "Fixture message 15");
    assert_eq!(fakes.draft(&target()).unwrap().body, "Fixture message 15");

    fakes.timeline_action(TimelineAction::CancelEdit { target: target() });
    assert!(
        next(&subscription).await.editing.is_none(),
        "取消以后还在编辑"
    );
    assert_eq!(fakes.draft(&target()).unwrap().body, "draft");

    fakes.timeline_action(TimelineAction::StartEdit {
        target: target(),
        item_id: "ui-fake-15".into(),
    });
    next(&subscription).await;
    fakes.timeline_action(TimelineAction::EditMessage {
        target: target(),
        event_id: "$ui-fake-15".into(),
        body: "Fixture edited".into(),
    });
    let batch = next(&subscription).await;
    assert!(batch.editing.is_none(), "确认以后还在编辑");
    let [TimelineDiff::Set { item, .. }] = batch.diffs.as_slice() else {
        panic!("确认以后这一条要换掉");
    };
    let TimelineContent::Message { message } = &item.content else {
        panic!("换掉的还是消息");
    };
    assert!(message.edited, "确认以后没标已编辑");
    let MessageContent::Text { body, .. } = &message.body else {
        panic!("正文还是文字");
    };
    assert_eq!(body, "Fixture edited");
    assert_eq!(fakes.draft(&target()).unwrap().body, "draft");
}

/// 编辑着又换一条改：恢复用的还是第一次进来之前的草稿（同真核心）。
#[tokio::test]
async fn switching_the_target_keeps_the_first_saved_draft() {
    let fakes = FakeTimelines::default();
    let subscription = fakes.subscribe(&target());
    next(&subscription).await;
    fakes.timeline_action(TimelineAction::SetDraft {
        target: target(),
        body: "draft".into(),
        reply_to: None,
    });
    fakes.timeline_action(TimelineAction::StartEdit {
        target: target(),
        item_id: "ui-fake-15".into(),
    });
    next(&subscription).await;
    fakes.timeline_action(TimelineAction::StartEdit {
        target: target(),
        item_id: "ui-fake-13".into(),
    });
    let switching = next(&subscription).await.editing.expect("换到另一条");
    assert_eq!(switching.item_id, "ui-fake-13");
    assert_eq!(switching.summary, "Fixture message 13");

    fakes.timeline_action(TimelineAction::CancelEdit { target: target() });
    next(&subscription).await;
    assert_eq!(fakes.draft(&target()).unwrap().body, "draft");
}

/// 编辑途中又打了字：退出时留着新打的，不拿进编辑前那份盖回去（同真核心）。
#[tokio::test]
async fn typing_during_an_edit_survives_cancel_and_confirm() {
    let fakes = FakeTimelines::default();
    let subscription = fakes.subscribe(&target());
    next(&subscription).await;
    fakes.timeline_action(TimelineAction::SetDraft {
        target: target(),
        body: "draft".into(),
        reply_to: None,
    });

    for confirm in [false, true] {
        fakes.timeline_action(TimelineAction::StartEdit {
            target: target(),
            item_id: "ui-fake-15".into(),
        });
        next(&subscription).await;
        fakes.timeline_action(TimelineAction::SetDraft {
            target: target(),
            body: "Fixture message 15 再补一句".into(),
            reply_to: None,
        });
        if confirm {
            fakes.timeline_action(TimelineAction::EditMessage {
                target: target(),
                event_id: "$ui-fake-15".into(),
                body: "Fixture message 15 再补一句".into(),
            });
        } else {
            fakes.timeline_action(TimelineAction::CancelEdit { target: target() });
        }
        next(&subscription).await;
        assert_eq!(
            fakes.draft(&target()).unwrap().body,
            "Fixture message 15 再补一句",
            "{}",
            if confirm { "确认" } else { "取消" }
        );
    }
}

/// 旁聊是按根消息开的另一份时间线：第一批是旁聊那四条，发出去的照片只落在旁聊里。
#[tokio::test]
async fn side_chat_is_its_own_timeline() {
    let fakes = FakeTimelines::default();
    let root = "$ui-fake-5";
    let main = fakes.subscribe(&target());
    next(&main).await;
    let side_target = TimelineTarget {
        room_id: ROOM.into(),
        thread_root: Some(root.into()),
    };
    let side = fakes.subscribe(&side_target);
    let first = next(&side).await;
    assert_eq!(first.viewport, wonton_protocol::TimelineViewport::Bottom);
    let [TimelineDiff::Reset { items }] = first.diffs.as_slice() else {
        panic!("旁聊的第一批是整条回复列表");
    };
    assert_eq!(
        items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["side-0", "side-1", "side-2", "side-3"]
    );
    assert_eq!(first.side_chat.unwrap().root_event_id, root);

    fakes.timeline_action(TimelineAction::SendImage {
        target: side_target.clone(),
        image: wonton_protocol::ImageUpload {
            filename: "photo.png".into(),
            mime_type: "image/png".into(),
            data: Vec::new(),
            width: None,
            height: None,
            thumbnail: None,
        },
        reply_to: None,
        caption: None,
    });
    let sent = next(&side).await;
    let [TimelineDiff::PushBack { item }] = sent.diffs.as_slice() else {
        panic!("旁聊里发出的照片回显一条");
    };
    let MessageContent::Text { body, .. } = &message(std::slice::from_ref(item), &item.id).body
    else {
        panic!("回显的是文字");
    };
    assert_eq!(body, "photo");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), main.next())
            .await
            .is_err(),
        "旁聊里发的照片落到了主对话"
    );
}
