//! 每个房间的草稿：只在本机，换房间、重启核心都还在，发出去以后才清掉。正在回复的消息和正文一起存（ADR-0073）。

use std::time::Duration;

use wonton_client::Action;
use wonton_protocol::{ComposerDraft, MessageAction, ReplyQuote, TimelineAction, TimelineTarget};

use super::accept_the_bots_invite;
use super::message_actions::{join_group_room, text_message};
use super::support::{Home, Phone, Timeline, WAIT};
use super::text;

/// 读一次草稿。房间还没同步到本机时核心回错，按空处理，交给调用方的循环等。
async fn composer(phone: &Phone, room_id: &str) -> Option<ComposerDraft> {
    phone
        .draft(TimelineTarget {
            room_id: room_id.to_owned(),
            thread_root: None,
        })
        .await
        .ok()
        .flatten()
}

async fn draft(phone: &Phone, room_id: &str) -> Option<String> {
    composer(phone, room_id).await.map(|draft| draft.body)
}

/// 等到草稿满足条件。写入在后台排队，读到想要的为止。
async fn until_composer(phone: &Phone, room_id: &str, want: Option<&ComposerDraft>) {
    let waiting = async {
        loop {
            if composer(phone, room_id).await.as_ref() == want {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    if tokio::time::timeout(WAIT, waiting).await.is_err() {
        let now = composer(phone, room_id).await;
        panic!("草稿一直不是 {want:?}，现在是 {now:?}");
    }
}

/// 等到草稿的正文是 `want`，不在回复。
async fn until_draft(phone: &Phone, room_id: &str, want: Option<&str>) {
    let want = want.map(|body| ComposerDraft {
        body: body.to_owned(),
        reply: None,
    });
    until_composer(phone, room_id, want.as_ref()).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn drafts_stay_per_room_and_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let first_bot = home.bot("hermes", "Hermes").await;
    let first = accept_the_bots_invite(&home, &phone, &first_bot)
        .await
        .to_string();
    let second_bot = home.bot("other", "Hermes").await;
    let second = accept_the_bots_invite(&home, &phone, &second_bot)
        .await
        .to_string();

    // 两个房间各有一份，换房间不互相盖。
    let set = |room: &str, body: &str| Action::Timeline {
        action: TimelineAction::SetDraft {
            target: TimelineTarget {
                room_id: room.to_owned(),
                thread_root: None,
            },
            body: body.to_owned(),
            reply_to: None,
        },
    };
    phone.dispatch(set(&first, "写给第一个房间")).unwrap();
    until_draft(&phone, &first, Some("写给第一个房间")).await;
    phone.dispatch(set(&second, "写给第二个房间")).unwrap();
    until_draft(&phone, &second, Some("写给第二个房间")).await;
    assert_eq!(
        draft(&phone, &first).await.as_deref(),
        Some("写给第一个房间")
    );

    // 连着敲很多次，先敲的内容不会盖掉后敲的。
    for count in 0..20 {
        phone
            .dispatch(set(&first, &format!("第一间第 {count} 个字")))
            .unwrap();
    }
    until_draft(&phone, &first, Some("第一间第 19 个字")).await;

    // 重启核心：两份草稿都还在。刚登录时房间列表可能还没同步回来，等它到。
    drop(phone);
    let phone = Phone::open(dir.path());
    phone.connect(&home);
    phone.signed_in().await;
    until_draft(&phone, &first, Some("第一间第 19 个字")).await;
    until_draft(&phone, &second, Some("写给第二个房间")).await;

    // 发出去以后清掉；别的房间不受影响。
    let mut timeline = Timeline::open(&phone, &first).await;
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: TimelineTarget {
                    room_id: first.clone(),
                    thread_root: None,
                },
                body: "第一间第 19 个字".to_owned(),
                reply_to: None,
            },
        })
        .unwrap();
    timeline
        .until("发出去的那句出现", |item| {
            text(item, "第一间第 19 个字")
        })
        .await;
    until_draft(&phone, &first, None).await;
    assert_eq!(
        draft(&phone, &second).await.as_deref(),
        Some("写给第二个房间"),
        "发别的房间不影响这个房间的草稿"
    );

    // 发送途中又打了字：草稿换成新的，发出去以后不能把它清掉。
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: TimelineTarget {
                    room_id: first.clone(),
                    thread_root: None,
                },
                body: "再发一句".to_owned(),
                reply_to: None,
            },
        })
        .unwrap();
    phone.dispatch(set(&first, "发送途中又打的")).unwrap();
    timeline
        .until("再发的那句出现", |item| text(item, "再发一句"))
        .await;
    assert_eq!(
        draft(&phone, &first).await.as_deref(),
        Some("发送途中又打的")
    );
}

/// 回复对象和正文一起进草稿：换房间、重启核心后还在，回复的发出去以后连回复一起清掉。正文空着、还在回复也算草稿。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_reply_stays_in_the_draft_until_it_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let first_id = join_group_room(&phone, &bot, "First").await;
    let (first, second) = (
        first_id.to_string(),
        join_group_room(&phone, &bot, "Second").await.to_string(),
    );
    let bot_room = bot.open_room(&first_id).await.expect("Bot 打开群聊");
    bot_room.send_text("要回复的话").await.expect("Bot 发文字");
    let mut timeline = Timeline::open(&phone, &first).await;
    let (_, original) = timeline
        .until("收到要回复的话", |item| {
            text_message(item, "要回复的话")
        })
        .await;
    let quote = original
        .actions
        .iter()
        .find_map(|action| match action {
            MessageAction::Reply { message } => Some(message.clone()),
            _ => None,
        })
        .expect("能回复");
    let set = |room: &str, body: &str, reply_to: Option<&str>| Action::Timeline {
        action: TimelineAction::SetDraft {
            target: TimelineTarget {
                room_id: room.to_owned(),
                thread_root: None,
            },
            body: body.to_owned(),
            reply_to: reply_to.map(str::to_owned),
        },
    };
    let replying = |body: &str| ComposerDraft {
        body: body.to_owned(),
        reply: Some(ReplyQuote::Ready {
            message: quote.clone(),
        }),
    };

    phone
        .dispatch(set(&first, "", Some(&quote.event_id)))
        .unwrap();
    until_composer(&phone, &first, Some(&replying(""))).await;
    phone
        .dispatch(set(&first, "草稿正文", Some(&quote.event_id)))
        .unwrap();
    phone.dispatch(set(&second, "另一间", None)).unwrap();
    until_composer(&phone, &first, Some(&replying("草稿正文"))).await;
    until_draft(&phone, &second, Some("另一间")).await;

    drop(timeline);
    drop(phone);
    let phone = Phone::open(dir.path());
    phone.connect(&home);
    phone.signed_in().await;
    until_composer(&phone, &first, Some(&replying("草稿正文"))).await;

    let mut timeline = Timeline::open(&phone, &first).await;
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: TimelineTarget {
                    room_id: first.clone(),
                    thread_root: None,
                },
                body: "草稿正文".to_owned(),
                reply_to: Some(quote.event_id.clone()),
            },
        })
        .unwrap();
    timeline
        .until("回复发出去", |item| {
            text_message(item, "草稿正文").filter(|(_, m)| m.event_id.is_some())
        })
        .await;
    until_composer(&phone, &first, None).await;
    until_draft(&phone, &second, Some("另一间")).await;
}
