//! 链接预览（ADR-0065）：发送框的预览和去掉、发送时附 `com.beeper.linkpreviews`（MSC4095 的 unstable
//! 字段）、收到带预览的直接用、不带的向核心取（同一网址在内存里去重）、错误按类别、每条消息只认第一个链接。

use serde_json::{Value, json};
use wonton_client::{
    Action, LinkPreviewReason, LinkPreviewState, MessageContent, TimelineContent, TimelineItem,
    TimelineMessage,
};
use wonton_matrix::ChatRoom;
use wonton_protocol::{
    LinkPreviewCard, LinkPreviewCardImage, LinkPreviewFailure, LinkPreviewImage, TimelineAction,
    TimelineTarget,
};

use super::PNG;
use super::message_actions::join_group_room;
use super::support::{Home, Phone, Timeline, WAIT};
use super::{accept_the_bots_invite, bot_receives};

/// 假核心给这个网址回的预览；`title` 区分两个网址，图片在核心的缓存里。
fn preview_body(final_url: &str, title: &str, image_id: Option<&str>) -> Value {
    json!({
        "url": final_url,
        "title": title,
        "description": "一段摘要",
        "site_name": "示例站",
        "image": image_id.map(|id| json!({
            "id": id,
            "mime_type": "image/png",
            "width": 2,
            "height": 2,
        })),
    })
}

/// 发送框该出的预览卡片：图片走核心的缓存。
fn ready_card(final_url: &str, title: &str, image_id: &str) -> LinkPreviewState {
    LinkPreviewState::Ready {
        card: LinkPreviewCard {
            url: final_url.to_owned(),
            title: Some(title.to_owned()),
            description: Some("一段摘要".to_owned()),
            site_name: Some("示例站".to_owned()),
            image: Some(LinkPreviewCardImage::Core {
                image: LinkPreviewImage {
                    id: image_id.to_owned(),
                    mime_type: "image/png".to_owned(),
                    width: Some(2),
                    height: Some(2),
                },
            }),
        },
    }
}

/// 发送框的预览等到和 `want` 一样为止。取预览在后台做，第一次读回就可能还没落定。
async fn until_composer_preview(phone: &Phone, room: &str, want: &LinkPreviewState) {
    let waiting = async {
        loop {
            let preview = phone
                .draft(TimelineTarget {
                    room_id: room.to_owned(),
                    thread_root: None,
                })
                .await
                .ok()
                .flatten()
                .and_then(|draft| draft.preview);
            if preview.as_ref() == Some(want) {
                return;
            }
        }
    };
    tokio::time::timeout(WAIT, waiting)
        .await
        .unwrap_or_else(|_| panic!("发送框的预览一直不是 {want:?}"));
}

/// 草稿的预览没有（被去掉了、链接没了），等到为止。
async fn until_composer_without_preview(phone: &Phone, room: &str) {
    let waiting = async {
        loop {
            let preview = phone
                .draft(TimelineTarget {
                    room_id: room.to_owned(),
                    thread_root: None,
                })
                .await
                .ok()
                .flatten()
                .and_then(|draft| draft.preview);
            if preview.is_none() {
                return;
            }
        }
    };
    tokio::time::timeout(WAIT, waiting)
        .await
        .expect("发送框的预览一直都在");
}

fn message_with_body<'a>(item: &'a TimelineItem, body: &str) -> Option<&'a TimelineMessage> {
    match &item.content {
        TimelineContent::Message { message } if matches!(&message.body, MessageContent::Text { body: text, .. } if text.contains(body)) => {
            Some(message)
        }
        _ => None,
    }
}

/// 时间线里这条消息的预览，等到它出现为止。
async fn until_message_preview(timeline: &mut Timeline, body: &str) -> LinkPreviewState {
    timeline
        .until("消息的预览", |item| {
            message_with_body(item, body).and_then(|message| message.preview.clone())
        })
        .await
}

/// Bot 发一条原文是这样的 `m.text`，额外字段原样带上。
async fn bot_sends(room: &ChatRoom, body: &str, previews: Option<Value>) {
    let mut content = json!({
        "msgtype": "m.text",
        "body": body,
    });
    if let Some(previews) = previews {
        content
            .as_object_mut()
            .unwrap()
            .insert("com.beeper.linkpreviews".to_owned(), previews);
    }
    room.send_raw_message(content).await.expect("Bot 发消息");
}

/// 把链接打进草稿。
fn set_draft(room: &str, body: String) -> Action {
    Action::Timeline {
        action: TimelineAction::SetDraft {
            target: TimelineTarget {
                room_id: room.to_owned(),
                thread_root: None,
            },
            body,
            reply_to: None,
        },
    }
}

/// 发这条文字。
fn send_text(room: String, body: String) -> Action {
    Action::Timeline {
        action: TimelineAction::SendText {
            target: TimelineTarget {
                room_id: room,
                thread_root: None,
            },
            body,
            reply_to: None,
        },
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_composer_fetches_the_preview_removes_it_and_sends_without_it() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;
    let room = room_id.to_string();
    let url = "https://example.com/a";
    home.set_link_preview(url, 200, preview_body(url, "第一个", Some("img")));
    home.set_link_preview_image("img", "image/png", PNG);

    // 草稿打一个链接，发送框出预览。
    let body = format!("看看 {url}");
    phone.dispatch(set_draft(&room, body.clone())).unwrap();
    until_composer_preview(&phone, &room, &ready_card(url, "第一个", "img")).await;

    // 点掉：这条草稿不再出。
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::DismissLinkPreview {
                target: TimelineTarget {
                    room_id: room.clone(),
                    thread_root: None,
                },
            },
        })
        .unwrap();
    until_composer_without_preview(&phone, &room).await;

    // 去掉了还发：事件里没有 `com.beeper.linkpreviews`。
    phone.dispatch(send_text(room.clone(), body)).unwrap();
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    bot_receives(&bot_room, "Bot 收到没带预览的消息", |message| {
        matches!(&message.body, wonton_matrix::MessageBody::Text(text) if text == "看看 https://example.com/a")
            .then(|| {
                assert!(
                    !message.custom.contains_key("com.beeper.linkpreviews"),
                    "去掉之后还发了预览：{:?}",
                    message.custom
                );
            })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_sent_message_carries_the_preview_with_an_uploaded_image() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let url = "https://example.com/a";
    home.set_link_preview(
        url,
        200,
        preview_body("https://example.com/final", "第一个", Some("img")),
    );
    home.set_link_preview_image("img", "image/png", PNG);

    // 不加密的群聊：图片直接是 mxc 地址，元数据照带。
    let plain_id = join_group_room(&phone, &bot, "预览").await;
    let plain = plain_id.to_string();
    let body = format!("看看 {url}");
    phone.dispatch(set_draft(&plain, body.clone())).unwrap();
    until_composer_preview(
        &phone,
        &plain,
        &ready_card("https://example.com/final", "第一个", "img"),
    )
    .await;
    phone.dispatch(send_text(plain.clone(), body)).unwrap();
    let bot_room = bot.open_room(&plain_id).await.expect("Bot 打开群聊");
    let bundled = bot_receives(&bot_room, "Bot 收到带预览的消息", |message| {
        message.custom.get("com.beeper.linkpreviews").cloned()
    })
    .await;
    let Value::Array(bundled) = bundled else {
        panic!("com.beeper.linkpreviews 该是数组：{bundled:?}");
    };
    let [entry] = bundled.as_slice() else {
        panic!("预览应该只有一条：{bundled:?}");
    };
    assert_eq!(entry["matched_url"], json!(url));
    assert_eq!(entry["og:url"], json!("https://example.com/final"));
    assert_eq!(entry["og:title"], json!("第一个"));
    assert_eq!(entry["og:description"], json!("一段摘要"));
    assert_eq!(entry["og:site_name"], json!("示例站"));
    assert_eq!(entry["og:image:type"], json!("image/png"));
    assert_eq!(entry["og:image:width"], json!(2));
    let mxc_plain = entry["og:image"].as_str().expect("图片是 mxc 地址");
    assert!(
        mxc_plain.starts_with("mxc://"),
        "不是 mxc 地址：{mxc_plain}"
    );

    // 加密房间：图片是加密文件，对方凭句柄取回解密后的原图。事件里是裸的 EncryptedFile，
    // 媒体句柄外面还要包一层 `"file"`（同 `previews::bundled_image`）。
    let encrypted_id = accept_the_bots_invite(&home, &phone, &bot).await;
    let encrypted = encrypted_id.to_string();
    let body = format!("看看 {url}");
    phone.dispatch(set_draft(&encrypted, body.clone())).unwrap();
    until_composer_preview(
        &phone,
        &encrypted,
        &ready_card("https://example.com/final", "第一个", "img"),
    )
    .await;
    phone.dispatch(send_text(encrypted, body)).unwrap();
    let encrypted_room = bot.open_room(&encrypted_id).await.expect("Bot 打开私聊");
    let bundled = bot_receives(
        &encrypted_room,
        "Bot 收到加密房间的预览",
        |message| message.custom.get("com.beeper.linkpreviews").cloned(),
    )
    .await;
    let encryption = &bundled[0]["beeper:image:encryption"];
    assert!(
        encryption["url"]
            .as_str()
            .unwrap_or_default()
            .starts_with("mxc://")
    );
    assert!(
        encryption["key"]["k"]
            .as_str()
            .is_some_and(|key| !key.is_empty())
    );
    assert!(encryption["iv"].as_str().is_some_and(|iv| !iv.is_empty()));
    assert!(encryption["hashes"]["sha256"].as_str().is_some());
    let token = wonton_matrix::encrypted_media_token(encryption).unwrap();
    let bytes = bot.download_media(&token).await.expect("Bot 解密预览图片");
    assert_eq!(bytes, PNG, "解密出来的不是原图");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_bundled_preview_is_used_and_the_core_is_not_asked() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = join_group_room(&phone, &bot, "带预览").await;
    let room = room_id.to_string();
    let mut timeline = Timeline::open(&phone, &room).await;
    let body = "看看 https://example.com/b";
    bot_sends(
        &bot.open_room(&room_id).await.expect("Bot 打开群聊"),
        body,
        Some(json!([{
            "matched_url": "https://example.com/b",
            "og:url": "https://example.com/b",
            "og:title": "事件里带的",
            "og:image": "mxc://wonton.localhost/abc",
            "og:image:type": "image/png",
        }])),
    )
    .await;

    let preview = until_message_preview(&mut timeline, "example.com/b").await;
    let card = match preview {
        LinkPreviewState::Ready { card } => card,
        other => panic!("该用事件里的预览，现在是 {other:?}"),
    };
    assert_eq!(card.title.as_deref(), Some("事件里带的"));
    assert!(matches!(
        card.image,
        Some(LinkPreviewCardImage::Matrix { .. })
    ));
    assert_eq!(home.link_preview_requests(), 0, "带了预览还向核心取了");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_message_without_a_preview_is_fetched_once_and_deduplicated() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = join_group_room(&phone, &bot, "没带预览").await;
    let room = room_id.to_string();
    home.set_link_preview(
        "https://example.com/b",
        200,
        preview_body("https://example.com/b", "核心取的", None),
    );
    let mut timeline = Timeline::open(&phone, &room).await;
    bot_sends(
        &bot.open_room(&room_id).await.expect("Bot 打开群聊"),
        "看看 https://example.com/b",
        None,
    )
    .await;

    let preview = until_message_preview(&mut timeline, "example.com/b").await;
    assert!(
        matches!(&preview, LinkPreviewState::Ready { card } if card.title.as_deref() == Some("核心取的")),
        "预览不对：{preview:?}"
    );
    assert_eq!(home.link_preview_requests(), 1);

    // 重开时间线（整条重画一遍）：同一网址在内存里去重，不再请求。
    drop(timeline);
    let mut again = Timeline::open(&phone, &room).await;
    again
        .until_items("重画后预览还在", |items| {
            items
                .iter()
                .find_map(|item| message_with_body(item, "example.com/b"))
                .filter(|message| message.preview.is_some())
                .map(|_| ())
        })
        .await;
    assert_eq!(home.link_preview_requests(), 1, "重画又请求了一次");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_failed_preview_shows_the_category_and_no_card() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = join_group_room(&phone, &bot, "取不到").await;
    let room = room_id.to_string();
    home.set_link_preview(
        "https://example.com/b",
        422,
        json!({"error": {"kind": "not_html", "message": "不是网页"}}),
    );
    let mut timeline = Timeline::open(&phone, &room).await;
    bot_sends(
        &bot.open_room(&room_id).await.expect("Bot 打开群聊"),
        "看看 https://example.com/b",
        None,
    )
    .await;

    let preview = until_message_preview(&mut timeline, "example.com/b").await;
    assert_eq!(
        preview,
        LinkPreviewState::Failed {
            reason: LinkPreviewReason::Upstream {
                reason: LinkPreviewFailure::NotHtml,
            }
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn only_the_first_link_of_a_message_gets_a_preview() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = join_group_room(&phone, &bot, "两个链接").await;
    let room = room_id.to_string();
    home.set_link_preview(
        "https://first.example/a",
        200,
        preview_body("https://first.example/a", "第一个", None),
    );
    home.set_link_preview(
        "https://second.example/b",
        200,
        preview_body("https://second.example/b", "第二个", None),
    );
    let mut timeline = Timeline::open(&phone, &room).await;
    bot_sends(
        &bot.open_room(&room_id).await.expect("Bot 打开群聊"),
        "https://first.example/a 和 https://second.example/b",
        None,
    )
    .await;

    let preview = until_message_preview(&mut timeline, "second.example").await;
    assert!(
        matches!(&preview, LinkPreviewState::Ready { card } if card.title.as_deref() == Some("第一个")),
        "出的是第二个链接的卡片：{preview:?}"
    );
    assert_eq!(home.link_preview_requests(), 1, "第二个链接也被取了");
}
