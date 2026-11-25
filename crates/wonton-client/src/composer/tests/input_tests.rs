use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn sending_and_adding_refuse_what_cannot_go_out() {
    let s = setup();
    assert!(matches!(
        s.link.send(ROOM, "  ", None, &[]),
        Err(CoreError::InvalidInput { .. })
    ));
    let mut not_image = photo("a.pdf");
    not_image.mime_type = "application/pdf".into();
    assert!(matches!(
        s.link.add_image(ROOM, not_image),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(matches!(
        s.link.add_file(ROOM, file_source("x"), " ".into(), None),
        Err(CoreError::InvalidInput { .. })
    ));

    for index in 0..10 {
        s.link
            .add_image(ROOM, photo(&format!("{index}.jpg")))
            .unwrap();
    }
    assert!(matches!(
        s.link.add_image(ROOM, photo("11.jpg")),
        Err(CoreError::InvalidInput { .. })
    ));
    assert_eq!(view(&s).max_attachments, 10);

    until("都在传", || s.fake.upload_calls() == 10).await;
    s.fake.finish(
        "0.jpg",
        Err(UplinkError::Failed(
            AttachmentFailure::Upstream,
            "坏了".into(),
        )),
    );
    until("失败", || {
        matches!(
            view(&s).attachments[0].state,
            AttachmentState::Failed { .. }
        )
    })
    .await;
    assert!(
        matches!(
            s.link.send(ROOM, "发", None, &[]),
            Err(CoreError::InvalidInput { .. })
        ),
        "附件区里有失败的不让发"
    );
    let failed = view(&s).attachments[0].id.clone();
    s.link.remove(ROOM, &failed).unwrap();
    s.link.send(ROOM, "发", None, &[]).unwrap();
    assert_eq!(view(&s).outgoing[0].attachments.len(), 9);
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_items_beyond_the_limit_are_refused() {
    let s = setup();
    let items = (0..11)
        .map(|index| super::NewItem::Image(photo(&format!("{index}.jpg"))))
        .collect();
    assert!(matches!(
        s.link.send_items(ROOM, "", items),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(view(&s).outgoing.is_empty());
}
