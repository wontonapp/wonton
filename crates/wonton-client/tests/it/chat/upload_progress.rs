//! 发图先有本地回显和核心换算的进度，最后变成已发送；有上传进度后取消，条目从时间线消失。

use wonton_client::{
    Action, ImageThumbnail, ImageUpload, MessageContent, SendState, TimelineContent, TimelineItem,
    TimelineMessage,
};

use super::accept_the_bots_invite;
use super::support::{Home, Phone, Timeline};

/// Tuwunel 的上限最低 10 MB，8 MB 能进队列又够上传走几步。内容对 Matrix 不透明。
const BIG: usize = 8 * 1024 * 1024;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn upload_progress_advances_to_sent_and_a_cancelled_image_disappears() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;
    let room = room_id.to_string();
    let mut timeline = Timeline::open(&phone, &room).await;

    // 一开始就以本地回显出现，没有事件 ID，还带着上传进度。
    phone
        .dispatch(Action::SendImage {
            room_id: room.clone(),
            image: upload("big.png"),
            reply_to: None,
        })
        .unwrap();
    let mut sending = false;
    let mut saw_progress = false;
    timeline
        .until("上传进度推进到已发送", |item| {
            let message = own_image(item, "big.png")?;
            match &message.send_state {
                SendState::Sending { progress } => {
                    assert!(message.event_id.is_none(), "还没发出去就不该有事件 ID");
                    sending = true;
                    if let Some(progress) = progress {
                        assert!((0.0..=1.0).contains(progress), "进度越界：{progress}");
                        saw_progress = true;
                    }
                    None
                }
                SendState::Sent => Some(()),
                SendState::Failed { message, .. } => panic!("发送失败：{message}"),
            }
        })
        .await;
    assert!(sending, "本地回显没有先以发送中出现");
    assert!(saw_progress, "已发送之前没见过上传进度");

    // 上传中的图可以取消：从时间线里消失。
    home.pause_uploads();
    phone
        .dispatch(Action::SendImage {
            room_id: room.clone(),
            image: upload("cancelled.png"),
            reply_to: None,
        })
        .unwrap();
    let echo = timeline
        .until("第二张已开始上传", |item| {
            own_image(item, "cancelled.png")
                .filter(|message| {
                    matches!(message.send_state, SendState::Sending { progress: Some(_) })
                })
                .map(|_| item.id.clone())
        })
        .await;
    phone
        .dispatch(Action::CancelSend {
            room_id: room.clone(),
            item_id: echo,
        })
        .unwrap();
    timeline
        .until_items("取消的图从时间线里消失", |items| {
            (!items
                .iter()
                .any(|item| own_image(item, "cancelled.png").is_some()))
            .then_some(())
        })
        .await;
    home.resume_uploads();
    assert!(
        phone.chat().problem.is_none(),
        "取消发送不应报错：{:?}",
        phone.chat().problem
    );
}

fn own_image<'a>(item: &'a TimelineItem, filename: &str) -> Option<&'a TimelineMessage> {
    match &item.content {
        TimelineContent::Message { message } if message.is_own => match &message.body {
            MessageContent::Image { image } if image.filename == filename => Some(message),
            _ => None,
        },
        _ => None,
    }
}

fn upload(filename: &str) -> ImageUpload {
    ImageUpload {
        filename: filename.to_owned(),
        mime_type: "image/png".to_owned(),
        data: vec![0xA5; BIG],
        width: None,
        height: None,
        thumbnail: Some(ImageThumbnail {
            mime_type: "image/png".to_owned(),
            data: b"a smaller preview".to_vec(),
            width: 1,
            height: 1,
        }),
    }
}
