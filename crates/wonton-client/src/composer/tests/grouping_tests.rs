use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn photos_and_text_go_out_as_one_group_only_after_every_photo_is_up() {
    let s = setup();
    s.link.add_image(ROOM, photo("a.jpg")).unwrap();
    s.link.add_image(ROOM, photo("b.jpg")).unwrap();
    until("两张都在传", || s.fake.upload_calls() == 2).await;
    assert_eq!(view(&s).attachments.len(), 2);

    s.link.send(ROOM, "哪张好看", None, &[]).unwrap();
    let pending = view(&s);
    assert!(pending.attachments.is_empty(), "附件区清空了");
    assert_eq!(pending.outgoing.len(), 1, "马上出现一条待发消息");
    assert_eq!(pending.outgoing[0].body, "哪张好看");
    assert!(matches!(
        pending.outgoing[0].state,
        SendState::Sending { .. }
    ));
    assert_eq!(*s.fake.drafts.lock().unwrap(), ["哪张好看"]);

    s.fake.finish("a.jpg", Ok(uploaded("a.jpg", true)));
    until("第一张传完", || {
        view(&s).outgoing[0].attachments[0].state == AttachmentState::Ready
    })
    .await;
    settle().await;
    assert_eq!(s.fake.groups(), 0, "第二张没传完，一条都不发");
    assert_eq!(
        view(&s).outgoing[0].state,
        SendState::Sending {
            progress: Some(0.75)
        }
    );

    s.fake.finish("b.jpg", Ok(uploaded("b.jpg", true)));
    until("交出去了", || s.fake.groups() == 1).await;
    {
        let groups = s.fake.groups.lock().unwrap();
        let group = &groups[0];
        assert_eq!(
            group.parts,
            [
                image(uploaded("a.jpg", true), None),
                image(uploaded("b.jpg", true), Some("哪张好看")),
            ]
        );
        let group_id = group.group_id.as_deref().expect("带分组标记");
        assert!(
            wonton_protocol::MessageGroup {
                id: group_id.to_owned(),
                index: 0,
                count: 2
            }
            .is_valid()
        );
    }
    until("待发消息由本地回显代替", || {
        view(&s).outgoing.is_empty()
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn files_are_granted_before_the_text_with_their_links_goes_out() {
    let s = setup();
    s.link.add_image(ROOM, photo("p.jpg")).unwrap();
    s.link
        .add_file(ROOM, file_source("report.pdf"), "report.pdf".into(), None)
        .unwrap();
    until("照片在传", || s.fake.upload_calls() == 1).await;
    until("文件在传", || {
        s.fake.transfers.borrow().active.len() == 1
    })
    .await;
    s.link
        .send(ROOM, "看看这个", Some("$event:example.org".into()), &[])
        .unwrap();

    s.fake.finish("p.jpg", Ok(uploaded("p.jpg", true)));
    settle().await;
    assert_eq!(s.fake.groups(), 0, "文件没传完，一条都不发");

    let file = FileId::from_bytes([7; 16]);
    s.fake.finish_file("t-report.pdf", &file);
    until("交出去了", || s.fake.groups() == 1).await;
    assert_eq!(
        *s.fake.log.lock().unwrap(),
        [format!("grant {file}"), "send".to_owned()],
        "先授权，再交出"
    );
    let groups = s.fake.groups.lock().unwrap();
    assert_eq!(
        groups[0].parts,
        [
            image(uploaded("p.jpg", true), None),
            Part::Text {
                body: format!("看看这个\n\n{}", file.link()),
                mentions: Vec::new()
            },
        ],
        "有文件时字和链接在最后一条，照片不带说明文字"
    );
    assert!(groups[0].group_id.is_some());
    assert_eq!(groups[0].reply_to.as_deref(), Some("$event:example.org"));
}
