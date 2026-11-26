use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn later_messages_wait_for_the_one_in_front() {
    let s = setup();
    s.link.add_image(ROOM, photo("a.jpg")).unwrap();
    s.link.send(ROOM, "第一条", None, &[]).unwrap();
    s.link.send(ROOM, "第二条", None, &[]).unwrap();
    until("在传", || s.fake.upload_calls() == 1).await;
    settle().await;
    assert_eq!(s.fake.groups(), 0, "只有字的也排在前面那条后面");
    assert_eq!(view(&s).outgoing.len(), 2);

    s.fake.finish("a.jpg", Ok(uploaded("a.jpg", true)));
    until("两条都交出去了", || s.fake.groups() == 2).await;
    let groups = s.fake.groups.lock().unwrap();
    assert_eq!(
        groups[0].parts,
        [image(uploaded("a.jpg", true), Some("第一条"))]
    );
    assert_eq!(
        groups[1].parts,
        [Part::Text {
            body: "第二条".into(),
            mentions: Vec::new()
        }]
    );
    assert_eq!(groups[1].group_id, None, "只有字的不带分组标记");
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_drops_the_whole_message_and_lets_the_next_go() {
    let s = setup();
    s.link.add_image(ROOM, photo("a.jpg")).unwrap();
    s.link
        .add_file(ROOM, file_source("b.pdf"), "b.pdf".into(), None)
        .unwrap();
    s.link.send(ROOM, "不要了", None, &[]).unwrap();
    s.link.send(ROOM, "这条要", None, &[]).unwrap();
    until("在传", || s.fake.upload_calls() == 1).await;
    until("文件在传", || {
        s.fake.transfers.borrow().active.len() == 1
    })
    .await;
    settle().await;

    let first = view(&s).outgoing[0].id.clone();
    s.link.cancel_outgoing(ROOM, &first).unwrap();
    until("后面那条交出去了", || s.fake.groups() == 1).await;
    assert_eq!(
        s.fake.groups.lock().unwrap()[0].parts,
        [Part::Text {
            body: "这条要".into(),
            mentions: Vec::new()
        }]
    );
    assert!(
        s.fake
            .log
            .lock()
            .unwrap()
            .contains(&"drop t-b.pdf".to_owned()),
        "文件的上传停下了"
    );
    until("待发消息清空", || view(&s).outgoing.is_empty()).await;
    assert!(matches!(
        s.link.cancel_outgoing(ROOM, &first),
        Err(CoreError::InvalidInput { .. })
    ));
}
