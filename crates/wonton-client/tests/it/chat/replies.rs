//! 回复（ADR-0073 第 2 节）：回复带引用，双方的时间线里都看得到引用的是哪条；原消息不在本地时先显示正在取，取到后
//! 换上，被撤回后跟着变；别的客户端的旧式回复去掉回退正文。

use serde_json::json;
use wonton_client::{
    Action, ImageThumbnail, ImageUpload, MessageContent, TimelineContent, TimelineItem,
    TimelineMessage,
};
use wonton_matrix::{MessageBody, Quoted, ReplyTo};
use wonton_protocol::{
    MessageAction, QuotedContent, QuotedMessage, ReplyQuote, TimelineAction, TimelineTarget,
};

use super::message_actions::{invite_to_group_room, join, text_message};
use super::support::{Home, Phone, Timeline};
use super::{PNG, THUMBNAIL, accept_the_bots_invite, bot_receives, test_image};

/// 菜单里 `Reply` 带的引用。
fn reply_quote(message: &TimelineMessage) -> QuotedMessage {
    message
        .actions
        .iter()
        .find_map(|action| match action {
            MessageAction::Reply { message } => Some(message.clone()),
            _ => None,
        })
        .expect("已发出的消息能回复")
}

/// 引用已经取到的回复里的引用。
fn ready_quote(message: &TimelineMessage) -> Option<QuotedMessage> {
    match &message.reply {
        Some(ReplyQuote::Ready { message }) => Some(message.clone()),
        _ => None,
    }
}

fn image_message(item: &TimelineItem, own: bool) -> Option<TimelineMessage> {
    match &item.content {
        TimelineContent::Message { message }
            if message.is_own == own
                && message.event_id.is_some()
                && matches!(message.body, MessageContent::Image { .. }) =>
        {
            Some(message.clone())
        }
        _ => None,
    }
}

fn upload() -> ImageUpload {
    ImageUpload {
        filename: "reply.png".to_owned(),
        mime_type: "image/png".to_owned(),
        data: PNG.to_vec(),
        width: Some(2),
        height: Some(2),
        thumbnail: Some(ImageThumbnail {
            mime_type: "image/png".to_owned(),
            data: THUMBNAIL.to_vec(),
            width: 1,
            height: 1,
        }),
    }
}

/// 回复文字和图片、用图片回复：自己这边和 Bot 那边的引用都是原消息的发送者和摘要。Element 的旧式回复去掉回退正文。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn replies_quote_the_original_on_both_sides() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;
    let room = room_id.to_string();
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    bot_room.send_text("原话").await.expect("Bot 发文字");
    bot_room.send_image(test_image()).await.expect("Bot 发图片");
    let mut timeline = Timeline::open(&phone, &room).await;
    let (_, original) = timeline
        .until("收到原话", |item| text_message(item, "原话"))
        .await;
    let quote = reply_quote(&original);
    assert_eq!(quote.sender_id, bot.user_id().as_str());
    assert_eq!(quote.sender_name, "Hermes");
    assert_eq!(
        quote.content,
        QuotedContent::Text {
            text: "原话".to_owned()
        }
    );
    let image = timeline
        .until("收到 Bot 的图片", |item| image_message(item, false))
        .await;
    let image_quote = reply_quote(&image);
    assert!(matches!(
        image_quote.content,
        QuotedContent::Image { thumbnail: Some(_) }
    ));

    text_reply_quotes_the_original(
        &phone,
        &mut timeline,
        &bot_room,
        &room,
        &quote,
        bot.user_id(),
    )
    .await;
    image_reply_quotes_the_original(&phone, &mut timeline, &bot_room, &image_quote).await;

    element_reply_drops_its_fallback(&bot_room, &mut timeline, &room, &quote).await;
}

/// 回一句文字：自己的回复带着引用发出去，Bot 看到的引用也对。
async fn text_reply_quotes_the_original(
    phone: &Phone,
    timeline: &mut Timeline,
    bot_room: &wonton_matrix::ChatRoom,
    room: &str,
    quote: &QuotedMessage,
    bot_user: &wonton_matrix::UserId,
) {
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: TimelineTarget {
                    room_id: room.to_string(),
                    thread_root: None,
                },
                body: "回一句".to_owned(),
                reply_to: Some(quote.event_id.clone()),
            },
        })
        .unwrap();
    let mine = timeline
        .until("自己的回复带着引用发出去", |item| {
            text_message(item, "回一句")
                .filter(|(_, m)| m.event_id.is_some())
                .and_then(|(_, m)| ready_quote(&m))
        })
        .await;
    assert_eq!(mine, quote.clone());
    let seen = bot_receives(bot_room, "Bot 看到的回复带着引用", |m| {
        match (&m.body, &m.reply) {
            (
                MessageBody::Text(text),
                Some(ReplyTo {
                    event_id,
                    quoted: Quoted::Ready(q),
                }),
            ) if text == "回一句" => Some((event_id.to_string(), q.clone())),
            _ => None,
        }
    })
    .await;
    assert_eq!(seen.0, quote.event_id);
    assert_eq!(seen.1.sender, *bot_user);
    assert_eq!(seen.1.body, MessageBody::Text("原话".to_owned()));
}

/// 回一张图片：自己的图片回复带着引用，Bot 看到的引用的是图片。
async fn image_reply_quotes_the_original(
    phone: &Phone,
    timeline: &mut Timeline,
    bot_room: &wonton_matrix::ChatRoom,
    image_quote: &QuotedMessage,
) {
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SendImage {
                target: TimelineTarget {
                    room_id: bot_room.room_id().to_string(),
                    thread_root: None,
                },
                image: upload(),
                reply_to: Some(image_quote.event_id.clone()),
            },
        })
        .unwrap();
    let mine = timeline
        .until("自己的图片回复带着引用", |item| {
            image_message(item, true).and_then(|m| ready_quote(&m))
        })
        .await;
    assert_eq!(mine, image_quote.clone());
    bot_receives(
        bot_room,
        "Bot 看到的图片回复引用的是图片",
        |m| match (&m.body, &m.reply) {
            (
                MessageBody::Image(_),
                Some(ReplyTo {
                    quoted: Quoted::Ready(q),
                    ..
                }),
            ) if matches!(q.body, MessageBody::Image(_)) => Some(()),
            _ => None,
        },
    )
    .await;
}

/// Element 发来的旧式回复（正文和 HTML 都带着引用的回退，真实的 Element Web 格式）：正文只剩回复的话，引用照常。
async fn element_reply_drops_its_fallback(
    bot_room: &wonton_matrix::ChatRoom,
    timeline: &mut Timeline,
    room: &str,
    quote: &QuotedMessage,
) {
    let (bot, original) = (&quote.sender_id, &quote.event_id);
    bot_room
        .send_raw_message(json!({
            "msgtype": "m.text",
            "body": format!("> <{bot}> 原话\n\nElement 的回复"),
            "format": "org.matrix.custom.html",
            "formatted_body": format!(
                "<mx-reply><blockquote><a href=\"https://matrix.to/#/{room}/{original}?via=wonton.localhost\">In reply to</a> <a href=\"https://matrix.to/#/{bot}\">{bot}</a><br>原话</blockquote></mx-reply>Element 的回复"
            ),
            "m.relates_to": { "m.in_reply_to": { "event_id": original } },
        }))
        .await
        .expect("Bot 发旧式回复");
    let element = timeline
        .until("旧式回复只剩回复的话", |item| {
            text_message(item, "Element 的回复").and_then(|(_, m)| ready_quote(&m))
        })
        .await;
    assert_eq!(&element, quote);
}

/// 原消息在翻页之前的地方：回复先显示正在取，取到后换成原消息；原消息被撤回后，引用跟着变成已撤回。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_quote_out_of_reach_loads_and_follows_a_redaction() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = invite_to_group_room(&phone, &bot, "Long").await;
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开群聊");
    let early = bot_room
        .send_text_event("很早的原话")
        .await
        .expect("Bot 发原话");
    // 原话和回复之间隔着比同步（房间订阅 20 条）和一页翻页（30 条）都多的消息：回复先到，原话不在本地。
    for count in 0..45 {
        bot_room
            .send_text(&format!("中间第 {count} 句"))
            .await
            .expect("Bot 发中间的话");
    }
    bot_room
        .send_reply("回复很早的那句", &early)
        .await
        .expect("Bot 回复");
    bot_receives(&bot_room, "Bot 的回复发出去", |m| {
        (m.body == MessageBody::Text("回复很早的那句".to_owned()) && m.event_id.is_some())
            .then_some(())
    })
    .await;

    join(&phone, &room_id).await;
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;
    let mut first = None;
    let loaded = timeline
        .until("引用的原话取到了", |item| {
            let (_, reply) = text_message(item, "回复很早的那句")?;
            first.get_or_insert_with(|| reply.reply.clone());
            ready_quote(&reply)
        })
        .await;
    assert!(
        matches!(first, Some(Some(ReplyQuote::Loading { ref event_id })) if *event_id == early.as_str()),
        "原话不在本地时应先显示正在取：{first:?}"
    );
    assert_eq!(loaded.sender_name, "Hermes");
    assert_eq!(loaded.sender_id, bot.user_id().as_str());
    assert_eq!(
        loaded.content,
        QuotedContent::Text {
            text: "很早的原话".to_owned()
        }
    );

    bot_room
        .redact(early.as_str(), "撤回")
        .await
        .expect("Bot 撤回原话");
    timeline
        .until("原话撤回后引用变成已撤回", |item| {
            let (_, reply) = text_message(item, "回复很早的那句")?;
            ready_quote(&reply).filter(|q| q.content == QuotedContent::Redacted)
        })
        .await;
}
