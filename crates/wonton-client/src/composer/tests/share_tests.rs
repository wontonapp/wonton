use super::*;

/// 系统分享进来的东西（ADR-0097）直接成一条待发消息，不带上这个房间附件区里放了一半的附件，也不动草稿。
#[tokio::test(flavor = "multi_thread")]
async fn sending_shared_items_leaves_the_tray_and_draft_alone() {
    let s = setup();
    s.link.add_image(ROOM, photo("tray.jpg")).unwrap();
    s.link
        .send_items(
            ROOM,
            " 看看这个 ",
            vec![
                super::NewItem::Image(photo("shared.jpg")),
                super::NewItem::File {
                    source: file_source("bill.pdf"),
                    name: "bill.pdf".into(),
                    mime_type: Some("application/pdf".into()),
                },
            ],
        )
        .unwrap();
    let pending = view(&s);
    assert_eq!(pending.attachments.len(), 1, "附件区里原来的那张还在");
    assert_eq!(pending.attachments[0].name, "tray.jpg");
    assert_eq!(pending.outgoing.len(), 1);
    assert_eq!(pending.outgoing[0].body, "看看这个");
    let names: Vec<_> = pending.outgoing[0]
        .attachments
        .iter()
        .map(|attachment| attachment.name.as_str())
        .collect();
    assert_eq!(names, ["shared.jpg", "bill.pdf"]);
    assert!(s.fake.drafts.lock().unwrap().is_empty(), "草稿没被当成用掉");

    until("照片在传", || {
        s.fake
            .upload_calls
            .lock()
            .unwrap()
            .iter()
            .any(|name| name == "shared.jpg")
    })
    .await;
    s.fake
        .finish("shared.jpg", Ok(uploaded("shared.jpg", false)));
    until("文件在传", || {
        s.fake
            .transfers
            .borrow()
            .active
            .iter()
            .any(|transfer| transfer.id == "t-bill.pdf")
    })
    .await;
    let file = FileId::from_bytes([9; 16]);
    s.fake.finish_file("t-bill.pdf", &file);
    until("交出去了", || s.fake.groups() == 1).await;
    let groups = s.fake.groups.lock().unwrap();
    assert_eq!(groups[0].parts.len(), 2, "照片一条、带链接和字的文字一条");
    assert!(groups[0].group_id.is_some());
}
