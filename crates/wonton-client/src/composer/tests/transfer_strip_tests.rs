use super::*;

/// 附件区的文件画在附件区和待发消息里，房间的传输条不再列；别的传输照列。附件区的上传不带房间，不拿掉的话每个房间都会列。
#[tokio::test(flavor = "multi_thread")]
async fn the_transfer_strip_leaves_out_the_trays_own_uploads() {
    let s = setup();
    s.link
        .add_file(ROOM, file_source("a.pdf"), "a.pdf".into(), None)
        .unwrap();
    s.link
        .add_file(ROOM, file_source("b.pdf"), "b.pdf".into(), None)
        .unwrap();
    until("文件在传", || {
        s.fake.transfers.borrow().active.len() == 2
    })
    .await;
    settle().await;
    s.link.send(ROOM, "", None, &[]).unwrap();
    s.link
        .add_file(ROOM, file_source("c.pdf"), "c.pdf".into(), None)
        .unwrap();
    until("第三个在传", || {
        s.fake.transfers.borrow().active.len() == 3
    })
    .await;
    settle().await;
    s.fake.transfers.send_modify(|view| {
        let mut other = view.active[0].clone();
        other.id = "download".into();
        other.direction = TransferDirection::Download;
        view.active.push(other);
    });

    let listed = s
        .link
        .hide_own_uploads(s.fake.transfers.borrow().for_room(ROOM, &[]));
    let ids: Vec<_> = listed
        .here
        .iter()
        .map(|transfer| transfer.id.as_str())
        .collect();
    assert_eq!(ids, ["download"]);
}
