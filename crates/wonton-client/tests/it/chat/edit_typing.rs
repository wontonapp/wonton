//! 编辑和正在输入的四个新动作要真的分派到核心：正在输入转给打开的房间；编辑先回无效操作，下一条 PR 才接。

use wonton_client::{Action, CoreError};
use wonton_protocol::{TimelineAction, TimelineTarget};

use super::message_actions::join_group_room;
use super::support::{Home, Phone, Timeline};

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
