//! SAS 验证完成后，加密历史原地解密、稳定 ID 不变；重启后仍已验证、历史仍可读。

use std::collections::BTreeMap;

use wonton_client::{
    Action, CoreError, DeviceVerification, MessageContent, RecoveryStatus, TimelineContent,
    TimelineItem, VerificationStage,
};

use super::super::accept_the_bots_encrypted_invite;
use super::super::support::{Phone, Timeline};
use super::{Devices, assert_recovered, compare, confirm_and_observe, flow_at};
use wonton_protocol::{TimelineAction, TimelineTarget};

const HISTORY: [&str; 3] = [
    "before pairing one",
    "before pairing two",
    "before pairing three",
];

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn sas_recovers_open_history_in_place_and_survives_restart() {
    let devices = Devices::new(true).await;
    let bot = devices.home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_encrypted_invite(&devices.home, &devices.first, &bot).await;
    let bot_room = bot.open_room(&room_id).await.unwrap();
    let mut first_timeline = Timeline::open(&devices.first, room_id.as_str()).await;
    for body in HISTORY {
        bot_room.send_text(body, &[]).await.unwrap();
        first_timeline
            .until("第一台手机读到加密历史", |item| {
                message_text(item).filter(|text| *text == body).map(|_| ())
            })
            .await;
    }
    let device_id = devices.connect_second().await;
    let mut timeline = open_history(&devices.second, room_id.as_str()).await;
    let stable_ids = timeline
        .until_items("新手机打开的历史无法解密", |items| {
            let ids: BTreeMap<_, _> = items
                .iter()
                .filter_map(|item| match &item.content {
                    TimelineContent::Message { message }
                        if message.body == MessageContent::Undecryptable =>
                    {
                        message
                            .event_id
                            .clone()
                            .map(|event| (event, item.id.clone()))
                    }
                    _ => None,
                })
                .collect();
            (ids.len() == HISTORY.len()).then_some(ids)
        })
        .await;
    let flow_id = compare(&devices.first, &devices.second).await;
    let stages =
        confirm_and_observe(&devices.first, &devices.second, &devices.home, &flow_id).await;
    assert!(
        stages.contains(&VerificationStage::FetchingKeys),
        "{stages:?}"
    );
    // 期限过了才到齐的话，`confirm_and_observe` 在 `KeysMissing` 就返回了，秘密到齐后流程才改成完成。
    if stages.last() != Some(&VerificationStage::Done) {
        flow_at(&devices.second, |stage| *stage == VerificationStage::Done).await;
    }
    assert_recovered(&devices.second);
    assert!(matches!(
        devices.second.dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::StartDeviceVerification
        }),
        Err(CoreError::NotReady { .. })
    ));
    timeline
        .until_items("同一时间线和稳定 ID 原地解密", |items| {
            let readable = history_ids(items)?;
            (readable == stable_ids).then_some(())
        })
        .await;
    drop(timeline);
    drop(first_timeline);
    let Devices {
        first,
        second,
        home,
        _first_dir,
        second_dir,
    } = devices;
    drop(second);
    let second = Phone::open(second_dir.path());
    assert_eq!(second.signed_in().await.1, device_id);
    second
        .wait_chat("重启仍然已验证且备份启用", |view| {
            (view.encryption.verification == DeviceVerification::Verified
                && view.encryption.recovery == RecoveryStatus::Enabled
                && !view.encryption.can_verify_with_other_device)
                .then_some(())
        })
        .await;
    let mut timeline = open_history(&second, room_id.as_str()).await;
    timeline
        .until_items("重启后旧消息仍可读", |items| {
            history_ids(items).map(|_| ())
        })
        .await;
    assert_recovered(&second);
    drop(timeline);
    drop(second);
    drop(first);
    drop(home);
}

async fn open_history(phone: &Phone, room_id: &str) -> Timeline {
    phone
        .wait_chat("新手机同步到已有私聊", |view| {
            view.rooms
                .iter()
                .any(|room| room.room_id == room_id)
                .then_some(())
        })
        .await;
    let timeline = Timeline::open(phone, room_id).await;
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::PaginateBackwards {
                target: TimelineTarget {
                    room_id: room_id.to_owned(),
                    thread_root: None,
                },
            },
        })
        .unwrap();
    timeline
}

fn message_text(item: &TimelineItem) -> Option<&str> {
    match &item.content {
        TimelineContent::Message { message } => match &message.body {
            MessageContent::Text { body, .. } => Some(body),
            _ => None,
        },
        _ => None,
    }
}

fn history_ids(items: &[TimelineItem]) -> Option<BTreeMap<String, String>> {
    let mut ids = BTreeMap::new();
    for body in HISTORY {
        let item = items.iter().find(|item| message_text(item) == Some(body))?;
        let TimelineContent::Message { message } = &item.content else {
            unreachable!()
        };
        ids.insert(message.event_id.clone()?, item.id.clone());
    }
    Some(ids)
}
