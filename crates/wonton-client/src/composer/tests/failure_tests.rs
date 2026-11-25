use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_photo_turns_the_message_red_and_retry_sends_it() {
    let s = setup();
    let id = s.link.add_image(ROOM, photo("big.jpg")).unwrap();
    s.link.send(ROOM, "", None, &[]).unwrap();
    until("在传", || s.fake.upload_calls() == 1).await;
    s.fake.finish(
        "big.jpg",
        Err(UplinkError::Failed(
            AttachmentFailure::TooLarge,
            "太大".into(),
        )),
    );
    until("整条标红", || {
        matches!(
            view(&s).outgoing.first().map(|outgoing| &outgoing.state),
            Some(SendState::Failed {
                recoverable: false,
                ..
            })
        )
    })
    .await;
    let outgoing = view(&s).outgoing[0].clone();
    assert_eq!(outgoing.attachments[0].id, id);
    assert!(matches!(
        outgoing.attachments[0].state,
        AttachmentState::Failed {
            reason: AttachmentFailure::TooLarge,
            ..
        }
    ));
    settle().await;
    assert_eq!(s.fake.groups(), 0);

    s.link.retry_outgoing(ROOM, &outgoing.id).unwrap();
    until("只重传失败的附件", || s.fake.upload_calls() == 2).await;
    assert!(matches!(
        view(&s).outgoing[0].state,
        SendState::Sending { .. }
    ));
    s.fake.finish("big.jpg", Ok(uploaded("big.jpg", true)));
    until("交出去了", || s.fake.groups() == 1).await;
    assert_eq!(
        s.fake.groups.lock().unwrap()[0].parts,
        [image(uploaded("big.jpg", true), None)],
        "没有字就没有说明文字"
    );
}

/// 失败是等主人处理的稳定态：迟到的进度和等联网都不能把它盖回上传中（否则发送按钮就不再被拦）；
/// 重试（`restart`）才把它改成上传中，之后照常收进度。
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_attachment_ignores_progress_and_offline_until_restart() {
    let s = setup();
    let id = s.link.add_image(ROOM, photo("big.jpg")).unwrap();
    s.link.fail_for_test(ROOM, &id).unwrap();
    until("标红", || {
        matches!(attachment(&s, &id), AttachmentState::Failed { .. })
    })
    .await;

    s.link.show(
        ROOM,
        &id,
        AttachmentState::Uploading {
            progress: Some(0.4),
        },
    );
    assert!(
        matches!(attachment(&s, &id), AttachmentState::Failed { .. }),
        "迟到的进度把失败盖回去了"
    );
    s.link.show(ROOM, &id, AttachmentState::Paused);
    assert!(
        matches!(attachment(&s, &id), AttachmentState::Failed { .. }),
        "等联网把失败盖回去了"
    );

    s.link.restart(ROOM, &id);
    until("重试后回到上传中", || {
        matches!(attachment(&s, &id), AttachmentState::Uploading { .. })
    })
    .await;
    s.link.show(
        ROOM,
        &id,
        AttachmentState::Uploading {
            progress: Some(0.5),
        },
    );
    assert_eq!(
        attachment(&s, &id),
        AttachmentState::Uploading {
            progress: Some(0.5),
        },
        "重试后的进度没收下"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_notification_reply_failure_survives_restart_and_retries_without_using_the_tray() {
    let s = setup();
    let tray = s.link.add_image(ROOM, photo("draft.jpg")).unwrap();
    *s.fake.send_error.lock().unwrap() = Some(UplinkError::Failed(
        AttachmentFailure::Rejected,
        "发送被拒绝".into(),
    ));
    let reply = serde_json::from_value(json!({
        "msgtype": "m.text", "body": "通知里的回复",
        "m.relates_to": { "m.in_reply_to": { "event_id": "$original:example.org" } }
    }))
    .unwrap();
    s.link.send_push_reply(ROOM, "通知里的回复", reply).unwrap();
    until("通知回复在时间线里显示失败", || {
        matches!(
            view(&s).outgoing.first().map(|outgoing| &outgoing.state),
            Some(SendState::Failed {
                recoverable: false,
                ..
            })
        )
    })
    .await;
    let failed = view(&s).outgoing[0].clone();
    assert_eq!(failed.body, "通知里的回复");
    assert!(failed.attachments.is_empty());
    assert_eq!(view(&s).attachments[0].id, tray);
    assert!(
        s.fake.drafts.lock().unwrap().is_empty(),
        "没有清掉输入框草稿"
    );

    let fake = Fake::new();
    let link = ComposerLink::new(
        tokio::runtime::Handle::current(),
        fake.clone(),
        s._dir.path(),
    );
    let restored = link.subscribe(ROOM);
    assert_eq!(
        restored.borrow().outgoing.as_slice(),
        std::slice::from_ref(&failed)
    );
    link.retry_outgoing(ROOM, &failed.id).unwrap();
    until("主人在App重试后交给Matrix", || fake.groups() == 1).await;
    until("待发由Matrix本地回显接替", || {
        restored.borrow().outgoing.is_empty()
    })
    .await;
    assert!(matches!(
        fake.groups.lock().unwrap()[0].parts.as_slice(),
        [Part::Reply(_)]
    ));
    assert_eq!(restored.borrow().attachments[0].id, tray);
    assert!(fake.drafts.lock().unwrap().is_empty());
}
