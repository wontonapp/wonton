//! 附件重传必须恢复整条待发消息，不能只把附件变绿、把文字和后续消息留在队首。

use super::*;
use wonton_protocol::FileFailure;

fn file_state(fake: &Fake, id: &str, state: TransferState) {
    fake.transfers.send_modify(|view| {
        let transfer = view.active.iter_mut().find(|seen| seen.id == id).unwrap();
        transfer.transferred_bytes = 40;
        transfer.state = state;
    });
}

fn failed(setup: &Setup) -> bool {
    matches!(
        view(setup).outgoing.first().map(|outgoing| &outgoing.state),
        Some(SendState::Failed {
            recoverable: false,
            ..
        })
    )
}

async fn interrupted_file() -> (Setup, String) {
    let s = setup();
    s.link.add_image(ROOM, photo("ready.jpg")).unwrap();
    let attachment = s
        .link
        .add_file(ROOM, file_source("report.pdf"), "report.pdf".into(), None)
        .unwrap();
    s.link
        .send(ROOM, "看看这个", Some("$event:example.org".into()))
        .unwrap();
    s.link.send(ROOM, "下一条", None).unwrap();
    until("两项开始上传", || {
        s.fake.upload_calls() == 1 && s.fake.transfers.borrow().active.len() == 1
    })
    .await;
    s.fake.finish("ready.jpg", Ok(uploaded("ready.jpg", true)));
    file_state(&s.fake, "t-report.pdf", TransferState::Paused);
    until("文件暂停、照片保留", || {
        let pending = view(&s);
        pending.outgoing[0].attachments[0].state == AttachmentState::Ready
            && pending.outgoing[0].attachments[1].state == AttachmentState::Paused
    })
    .await;
    let pending = view(&s);
    assert!(matches!(
        pending.outgoing[0].state,
        SendState::Sending { .. }
    ));
    assert_eq!(pending.outgoing[0].body, "看看这个");
    assert_eq!(pending.outgoing[1].body, "下一条");
    assert_eq!(s.fake.groups(), 0);

    file_state(&s.fake, "t-report.pdf", TransferState::Running);
    until("恢复进度", || {
        matches!(
            view(&s).outgoing[0].attachments[1].state,
            AttachmentState::Uploading { progress: Some(_) }
        )
    })
    .await;
    file_state(
        &s.fake,
        "t-report.pdf",
        TransferState::Failed {
            reason: FileFailure::Protocol,
            message: "unclassified upload response".into(),
        },
    );
    until("等待主人重传", || failed(&s)).await;
    (s, attachment)
}

#[tokio::test]
async fn retrying_an_interrupted_file_sends_its_text_once_before_later_messages() {
    let (s, attachment) = interrupted_file().await;
    let outgoing = view(&s).outgoing[0].clone();
    assert!(matches!(
        outgoing.attachments[1].state,
        AttachmentState::Failed {
            reason: AttachmentFailure::Upstream,
            ..
        }
    ));
    assert_eq!(s.fake.groups(), 0);

    s.link.retry(ROOM, &attachment).unwrap();
    s.link.retry(ROOM, &attachment).unwrap();
    s.link.retry_outgoing(ROOM, &outgoing.id).unwrap();
    until("仅文件重新上传", || {
        s.fake.file_calls.lock().unwrap().len() == 2
    })
    .await;
    assert_eq!(s.fake.upload_calls(), 1, "传好的照片不重传");
    let retrying = view(&s);
    assert_eq!(retrying.outgoing[0].id, outgoing.id);
    assert_eq!(retrying.outgoing[0].body, outgoing.body);
    assert!(matches!(
        retrying.outgoing[0].state,
        SendState::Sending { .. }
    ));
    assert_eq!(s.fake.groups(), 0, "重传未完成不能发正文或后续消息");

    let file = FileId::from_bytes([7; 16]);
    s.fake.finish_file("t-report.pdf-2", &file);
    until("两条按顺序交出", || view(&s).outgoing.is_empty()).await;
    assert_eq!(
        *s.fake.file_calls.lock().unwrap(),
        ["report.pdf", "report.pdf"]
    );
    assert_eq!(
        *s.fake.log.lock().unwrap(),
        [
            "drop t-report.pdf".to_owned(),
            format!("grant {file}"),
            "send".into(),
            "send".into()
        ]
    );
    let groups = s.fake.groups.lock().unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(
        groups[0].parts,
        [
            image(uploaded("ready.jpg", true), None),
            Part::Text(format!("看看这个\n\n{}", file.link()))
        ]
    );
    assert_eq!(groups[0].reply_to.as_deref(), Some("$event:example.org"));
    assert_eq!(groups[0].skip, 0);
    assert!(groups[0].group_id.is_some());
    assert_eq!(groups[1].parts, [Part::Text("下一条".into())]);
    assert_eq!(groups[1].group_id, None);
}

#[tokio::test]
async fn individually_retried_attachments_still_wait_for_every_failed_item() {
    let s = setup();
    let first = s.link.add_image(ROOM, photo("a.jpg")).unwrap();
    let second = s.link.add_image(ROOM, photo("b.jpg")).unwrap();
    s.link.send(ROOM, "两张", None).unwrap();
    until("两张在传", || s.fake.upload_calls() == 2).await;
    for name in ["a.jpg", "b.jpg"] {
        s.fake.finish(
            name,
            Err(UplinkError::Failed(
                AttachmentFailure::Upstream,
                "failed".into(),
            )),
        );
    }
    until("两张都失败", || {
        failed(&s)
            && view(&s).outgoing[0]
                .attachments
                .iter()
                .all(|item| matches!(item.state, AttachmentState::Failed { .. }))
    })
    .await;
    let outgoing = view(&s).outgoing[0].id.clone();

    s.link.retry(ROOM, &first).unwrap();
    until("第一张重传", || s.fake.upload_calls() == 3).await;
    s.fake.finish("a.jpg", Ok(uploaded("a.jpg", true)));
    until("第一张就绪、第二张仍阻塞发送", || {
        failed(&s) && view(&s).outgoing[0].attachments[0].state == AttachmentState::Ready
    })
    .await;
    assert_eq!(s.fake.groups(), 0);
    s.link.retry(ROOM, &second).unwrap();
    until("第二张重传", || s.fake.upload_calls() == 4).await;
    s.fake.finish(
        "b.jpg",
        Err(UplinkError::Failed(
            AttachmentFailure::Rejected,
            "rejected".into(),
        )),
    );
    until("重传被拒仍保留内容", || failed(&s)).await;
    assert_eq!(view(&s).outgoing[0].body, "两张");
    assert!(matches!(
        view(&s).outgoing[0].attachments[1].state,
        AttachmentState::Failed {
            reason: AttachmentFailure::Rejected,
            ..
        }
    ));
    assert_eq!(s.fake.groups(), 0);

    s.link.retry_outgoing(ROOM, &outgoing).unwrap();
    until("整条重试只传仍失败的一张", || {
        s.fake.upload_calls() == 5
    })
    .await;
    s.fake.finish("b.jpg", Ok(uploaded("b.jpg", true)));
    until("整组交出", || view(&s).outgoing.is_empty()).await;
    let calls = s.fake.upload_calls.lock().unwrap();
    assert_eq!(calls.iter().filter(|name| *name == "a.jpg").count(), 2);
    assert_eq!(calls.iter().filter(|name| *name == "b.jpg").count(), 3);
    let groups = s.fake.groups.lock().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0].parts,
        [
            image(uploaded("a.jpg", true), None),
            image(uploaded("b.jpg", true), Some("两张"))
        ]
    );
}

#[tokio::test]
async fn retrying_a_tray_attachment_does_not_release_another_failed_message() {
    let s = setup();
    s.link.add_image(ROOM, photo("outgoing.jpg")).unwrap();
    s.link.send(ROOM, "仍在等待", None).unwrap();
    let tray = s.link.add_image(ROOM, photo("tray.jpg")).unwrap();
    until("两张在传", || s.fake.upload_calls() == 2).await;
    for name in ["outgoing.jpg", "tray.jpg"] {
        s.fake.finish(
            name,
            Err(UplinkError::Failed(
                AttachmentFailure::Upstream,
                "failed".into(),
            )),
        );
    }
    until("附件区和待发都失败", || {
        failed(&s)
            && matches!(
                view(&s).attachments[0].state,
                AttachmentState::Failed { .. }
            )
    })
    .await;
    let outgoing = view(&s).outgoing[0].clone();
    s.link.retry(ROOM, &tray).unwrap();
    until("附件区开始重传", || s.fake.upload_calls() == 3).await;
    s.fake.finish("tray.jpg", Ok(uploaded("tray.jpg", true)));
    until("附件区就绪", || {
        view(&s).attachments[0].state == AttachmentState::Ready
    })
    .await;
    assert_eq!(
        view(&s).outgoing.as_slice(),
        std::slice::from_ref(&outgoing)
    );
    assert_eq!(s.fake.groups(), 0);
    s.link.cancel_outgoing(ROOM, &outgoing.id).unwrap();
    assert_eq!(s.fake.groups(), 0, "附件区不会自动发送");
    s.link.send(ROOM, "新消息", None).unwrap();
    until("明确发送后交出", || view(&s).outgoing.is_empty()).await;
    let groups = s.fake.groups.lock().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0].parts,
        [image(uploaded("tray.jpg", true), Some("新消息"))]
    );
}
