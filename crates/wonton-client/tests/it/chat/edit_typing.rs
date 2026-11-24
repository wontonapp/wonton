//! 编辑和正在输入的四个新动作要真的分派到核心：正在输入由核心节流、自己收；编辑先回无效操作，下一条 PR 才接。

use std::time::Duration;

use futures_util::StreamExt;
use wonton_client::{Action, CoreError};
use wonton_protocol::{TimelineAction, TimelineTarget};

use super::message_actions::join_group_room;
use super::support::{Home, Phone, Timeline, WAIT};

fn target(room: &str) -> TimelineTarget {
    TimelineTarget {
        room_id: room.to_owned(),
        thread_root: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn typing_goes_to_the_room_and_edit_is_still_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room = join_group_room(&phone, &bot, "群聊").await.to_string();
    let _timeline = Timeline::open(&phone, &room).await;

    for typing in [true, false] {
        phone
            .dispatch(Action::Timeline {
                action: TimelineAction::SetTyping {
                    target: target(&room),
                    typing,
                },
            })
            .expect("报告正在输入");
    }

    // 没订阅的房间里回的是“未就绪”，不是“无效操作”：正在输入已经走到打开的房间那一步。
    let error = phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SetTyping {
                target: target("!unsubscribed:home.example"),
                typing: true,
            },
        })
        .unwrap_err();
    assert!(matches!(error, CoreError::NotReady { .. }), "{error:?}");

    // 编辑还没接：订阅着也还是无效操作。
    let error = phone
        .dispatch(Action::Timeline {
            action: TimelineAction::StartEdit {
                target: target(&room),
                item_id: "x".to_owned(),
            },
        })
        .unwrap_err();
    assert!(matches!(error, CoreError::InvalidInput { .. }), "{error:?}");
}

/// 正在输入在核心里节流和收尾：接着敲的几下不再发，壳报告停了马上收，停手不管也自己收；发送消息时先收。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn typing_is_throttled_and_stops_by_itself() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    let (owner, _) = phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = join_group_room(&phone, &bot, "群聊").await;
    let room = room_id.to_string();
    let _timeline = Timeline::open(&phone, &room).await;
    let mut notices = Box::pin(
        bot.typing_notifications(&room_id)
            .expect("Bot 观察正在输入"),
    );

    let report = |typing: bool| {
        phone
            .dispatch(Action::Timeline {
                action: TimelineAction::SetTyping {
                    target: target(&room),
                    typing,
                },
            })
            .expect("报告正在输入");
    };

    // 第一下马上发出去。
    report(true);
    let users = tokio::time::timeout(WAIT, notices.next())
        .await
        .expect("没有发出正在输入")
        .expect("流断掉了");
    assert!(users.iter().any(|user| user.as_str() == owner), "{users:?}");

    // 节流期内接着敲：不再发。
    for _ in 0..3 {
        report(true);
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    let again = tokio::time::timeout(Duration::from_millis(1500), notices.next()).await;
    assert!(again.is_err(), "节流期内又发了一次");

    // 壳报告停了：马上收。
    report(false);
    let users = tokio::time::timeout(Duration::from_secs(15), notices.next())
        .await
        .expect("壳报告停了没有收")
        .expect("流断掉了");
    assert!(users.is_empty(), "{users:?}");

    // 停手不管也自己收：中间那次续发跳过，只等“停了”。给 40 秒：负载高时同步会断一会儿再自己接上。
    report(true);
    tokio::time::timeout(WAIT, notices.next())
        .await
        .expect("没有发出正在输入")
        .expect("流断掉了");
    tokio::time::timeout(Duration::from_secs(40), async {
        loop {
            let users = notices.next().await.expect("流断掉了");
            if users.is_empty() {
                break;
            }
        }
    })
    .await
    .expect("停手 5 秒后没有自己收");

    // 发送消息时先收。
    report(true);
    tokio::time::timeout(WAIT, notices.next())
        .await
        .expect("没有发出正在输入")
        .expect("流断掉了");
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: target(&room),
                body: "发一条".to_owned(),
                reply_to: None,
                mentions: Vec::new(),
            },
        })
        .expect("发消息");
    let users = tokio::time::timeout(Duration::from_secs(15), notices.next())
        .await
        .expect("发送消息没有收掉正在输入")
        .expect("流断掉了");
    assert!(users.is_empty(), "{users:?}");
}
