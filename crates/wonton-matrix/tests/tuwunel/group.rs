//! 一次发送里的几条事件（ADR-0094）：照片先传完，再一起交给发送队列；对方收到的每条都带同一个分组标记，字是最后一张的说明文字。

use wonton_matrix::{GroupPart, ImageThumbnail, ImageUpload, MessageBody};

use crate::{
    chat::direct_chat,
    support::{TestServer, eventually, random_bytes, unique, wait_for_room},
};

const GROUP: &str = "0123456789abcdef0123456789abcdef";

fn photo(name: &str) -> ImageUpload {
    ImageUpload {
        filename: name.to_owned(),
        mime_type: "image/jpeg".to_owned(),
        data: random_bytes(64 * 1024),
        width: Some(640),
        height: Some(480),
        thumbnail: Some(ImageThumbnail {
            mime_type: "image/jpeg".to_owned(),
            data: random_bytes(4 * 1024),
            width: 320,
            height: 240,
        }),
    }
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_group_of_photos_arrives_with_one_marker_and_the_caption_on_the_last() {
    let server = TestServer::start().await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let bob = server.login(&unique("bob"), "Bob 的手机").await;
    alice.start_sync().await.expect("Alice 开始同步");
    bob.start_sync().await.expect("Bob 开始同步");
    let room_id = direct_chat(&alice, &bob, true).await;
    wait_for_room(&bob, "私聊是加密的", &room_id, |row| row.is_encrypted).await;
    assert!(
        alice.room_is_encrypted(&room_id).await.expect("查房间加密"),
        "加密房间"
    );

    let second = photo("b.jpg");
    let mut uploaded = Vec::new();
    for image in [photo("a.jpg"), second.clone()] {
        let image = alice
            .upload_image(&room_id, image, |_, _| {})
            .await
            .expect("上传照片");
        assert!(image.is_encrypted(), "加密房间里的照片加密后传");
        uploaded.push(image);
    }
    let caption_on = uploaded.len() - 1;
    let parts = uploaded
        .into_iter()
        .enumerate()
        .map(|(index, image)| GroupPart::Image {
            image,
            caption: (index == caption_on).then(|| "哪张好看".to_owned()),
        })
        .collect();
    let handed = std::sync::Mutex::new(Vec::new());
    alice
        .send_group(&room_id, parts, Some(GROUP), None, 0, |count| {
            handed.lock().unwrap().push(count);
        })
        .await
        .expect("交给发送队列");
    assert_eq!(*handed.lock().unwrap(), [1, 2], "每交出一条报一次");

    let bob_room = bob.open_room(&room_id).await.expect("Bob 打开私聊");
    let images = eventually(
        "Bob 收到两张照片",
        bob_room.message_updates().await,
        |messages| {
            let images: Vec<_> = messages
                .iter()
                .filter_map(|message| match &message.body {
                    MessageBody::Image(image) => Some((message.group.clone(), image.clone())),
                    _ => None,
                })
                .collect();
            (images.len() == 2).then_some(images)
        },
    )
    .await;
    for (index, (group, image)) in images.iter().enumerate() {
        let group = group.as_ref().expect("每条都带分组标记");
        assert_eq!(group.id, GROUP);
        assert_eq!(group.count, 2);
        assert_eq!(group.index as usize, index, "按顺序到");
        let caption = (index == 1).then_some("哪张好看");
        assert_eq!(image.caption.as_deref(), caption, "字是最后一张的说明文字");
    }
    let downloaded = bob
        .download_media(&images[1].1.media_token())
        .await
        .expect("下载并解密第二张");
    assert!(downloaded == second.data, "解密后的照片与原图不一致");
}
