use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn uploads_pause_offline_and_resume_when_back() {
    let s = setup();
    s.fake.online.send_replace(false);
    s.link.add_image(ROOM, photo("a.jpg")).unwrap();
    until("暂停", || {
        view(&s).attachments[0].state == AttachmentState::Paused
    })
    .await;

    // 等联网（Paused）不算失败：这时照样点得了发送，字排在附件后面等。
    s.link.send(ROOM, "先发字", None, &[]).unwrap();
    assert_eq!(view(&s).outgoing.len(), 1, "暂停时发送被拒了");

    s.fake.online.send_replace(true);
    until("连上后接着传", || s.fake.upload_calls() == 1).await;
    until("显示进度", || {
        // 发送以后附件挪进待发消息，附件区是空的；进度照样写得进去（`item_mut` 两边都找）。
        view(&s).outgoing[0].attachments[0].state
            == AttachmentState::Uploading {
                progress: Some(0.5),
            }
    })
    .await;
    s.fake.finish("a.jpg", Ok(uploaded("a.jpg", true)));
    // 传完马上被那条待发消息带走，附件区清空、字发出去。
    until("传完、字发出去", || view(&s).outgoing.is_empty()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_photo_is_uploaded_again_when_the_room_became_encrypted() {
    let s = setup();
    s.link.add_image(ROOM, photo("a.jpg")).unwrap();
    until("在传", || s.fake.upload_calls() == 1).await;
    s.fake.finish("a.jpg", Ok(uploaded("a.jpg", false)));
    until("传完", || {
        view(&s).attachments[0].state == AttachmentState::Ready
    })
    .await;

    *s.fake.encrypted.lock().unwrap() = true;
    s.link.send(ROOM, "", None, &[]).unwrap();
    until("加密重传", || s.fake.upload_calls() == 2).await;
    settle().await;
    assert_eq!(s.fake.groups(), 0, "不把明文照片发进加密房间");
    s.fake.finish("a.jpg", Ok(uploaded("a.jpg", true)));
    until("交出去了", || s.fake.groups() == 1).await;
    assert_eq!(
        s.fake.groups.lock().unwrap()[0].parts,
        [image(uploaded("a.jpg", true), None)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_handover_cut_short_continues_where_it_stopped() {
    let s = setup();
    s.link.add_image(ROOM, photo("a.jpg")).unwrap();
    s.link.add_image(ROOM, photo("b.jpg")).unwrap();
    until("在传", || s.fake.upload_calls() == 2).await;
    *s.fake.cut_after.lock().unwrap() = Some(1);
    s.link.send(ROOM, "两张", None, &[]).unwrap();
    s.fake.finish("a.jpg", Ok(uploaded("a.jpg", true)));
    s.fake.finish("b.jpg", Ok(uploaded("b.jpg", true)));
    until("交了两次", || s.fake.groups() == 2).await;
    let groups = s.fake.groups.lock().unwrap();
    assert_eq!(groups[0].skip, 0);
    assert_eq!(groups[1].skip, 1, "交出去的那条不再交一次");
    assert_eq!(groups[0].group_id, groups[1].group_id, "还是同一组");
}

/// 字节都交给服务器（tusd 回 204）以后不再报比例：状态是“已保存，等待送达”，直到服务器收下事件。
/// 中继限速时这一段能挂几分钟，停在 100% 会让人以为还在传。
#[tokio::test(flavor = "multi_thread")]
async fn a_delivered_file_waits_without_a_percentage_until_the_message_lands() {
    let s = setup();
    let (release, gate) = oneshot::channel();
    *s.fake.send_gate.lock().unwrap() = Some(gate);
    s.link
        .add_file(ROOM, file_source("report.pdf"), "report.pdf".into(), None)
        .unwrap();
    s.link.send(ROOM, "看看这个", None, &[]).unwrap();
    until("文件在传", || {
        s.fake.transfers.borrow().active.len() == 1
    })
    .await;
    until("在传时有比例", || {
        matches!(
            view(&s).outgoing.first().map(|outgoing| &outgoing.state),
            Some(SendState::Sending { progress: Some(_) })
        )
    })
    .await;

    // 204 只说明字节收完了：文件就绪、授权也做了，事件还卡在发送队列里。
    s.fake
        .finish_file("t-report.pdf", &FileId::from_bytes([7; 16]));
    until("传完换成等待送达", || {
        view(&s).outgoing.first().map(|outgoing| &outgoing.state)
            == Some(&SendState::Sending { progress: None })
    })
    .await;
    until("授权完、交给发送队列", || {
        s.fake.log.lock().unwrap().len() == 2
    })
    .await;
    {
        let log = s.fake.log.lock().unwrap();
        assert!(log[0].starts_with("grant "), "{log:?}");
        assert_eq!(log[1], "send");
    }

    release.send(()).unwrap();
    until("落地后清掉", || view(&s).outgoing.is_empty()).await;
    assert_eq!(s.fake.groups(), 1);
}
