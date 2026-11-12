//! 发送失败的消息（`SendState` 文档写的行为，#86）：服务器拒收的一条停在时间线里，挡住之后发的；主人取消它以后，
//! 它从时间线里消失，排在它后面的发出去。

use wonton_client::{
    Action, MessageContent, SendState, TimelineContent, TimelineItem, TimelineMessage,
};
use wonton_matrix::MessageBody;

use super::support::{Home, Phone, Timeline};
use super::{accept_the_bots_invite, bot_receives};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_owner_cancels_a_rejected_message_and_the_next_one_goes_out() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;
    let room = room_id.to_string();
    let mut timeline = Timeline::open(&phone, &room).await;

    // 加密后的事件远超 65535 字节，Tuwunel 拒收（413 M_TOO_LARGE）。
    let too_long = "字".repeat(30_000);
    phone
        .dispatch(Action::SendText {
            room_id: room.clone(),
            body: too_long.clone(),
            reply_to: None,
        })
        .unwrap();
    let rejected = timeline
        .until("太长的那条被拒收", |item| {
            own_text(item, &too_long)
                .filter(|message| {
                    matches!(
                        message.send_state,
                        SendState::Failed {
                            recoverable: false,
                            ..
                        }
                    )
                })
                .map(|_| item.id.clone())
        })
        .await;
    phone
        .dispatch(Action::SendText {
            room_id: room.clone(),
            body: "排在后面的".to_owned(),
            reply_to: None,
        })
        .unwrap();

    phone
        .dispatch(Action::CancelSend {
            room_id: room.clone(),
            item_id: rejected,
        })
        .unwrap();
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    bot_receives(&bot_room, "Bot 收到排在后面的", |message| {
        (message.body == MessageBody::Text("排在后面的".to_owned())).then_some(())
    })
    .await;
    timeline
        .until_items("取消的那条从时间线里消失", |items| {
            (!items.iter().any(|item| own_text(item, &too_long).is_some())).then_some(())
        })
        .await;
    assert!(
        phone.chat().problem.is_none(),
        "取消发送不应报错：{:?}",
        phone.chat().problem
    );
}

fn own_text<'a>(item: &'a TimelineItem, expected: &str) -> Option<&'a TimelineMessage> {
    match &item.content {
        TimelineContent::Message { message }
            if message.is_own
                && matches!(&message.body, MessageContent::Text { body, .. } if body == expected) =>
        {
            Some(message)
        }
        _ => None,
    }
}
