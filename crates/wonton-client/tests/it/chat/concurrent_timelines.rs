//! Independent conversation subscriptions use the real Matrix fixture, without native windows.

use wonton_client::{Action, TimelineScroll, TimelineViewport};
use wonton_protocol::{PanelScope, TimelineAction, TimelineTarget};

use super::support::{Home, Phone, Timeline};
use super::{accept_the_bots_invite, text};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn concurrent_rooms_and_duplicate_subscriptions_keep_receiving_after_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let first_bot = home.bot("hermes", "Hermes").await;
    let second_bot = home.bot("codex", "Codex").await;
    let first_id = accept_the_bots_invite(&home, &phone, &first_bot).await;
    let second_id =
        super::message_actions::join_group_room(&phone, &second_bot, "Second conversation").await;
    let first_room = first_bot.open_room(&first_id).await.unwrap();
    let second_room = second_bot.open_room(&second_id).await.unwrap();
    let mut first = Timeline::open(&phone, first_id.as_str()).await;
    let mut duplicate = Timeline::open(&phone, first_id.as_str()).await;
    let mut second = Timeline::open(&phone, second_id.as_str()).await;
    first_room.send_text("first live").await.unwrap();
    second_room.send_text("second live").await.unwrap();
    first
        .until("first room updates", |item| text(item, "first live"))
        .await;
    duplicate
        .until("duplicate updates", |item| text(item, "first live"))
        .await;
    second
        .until("second room updates", |item| text(item, "second live"))
        .await;
    first.report_scroll(TimelineScroll::Away).await;
    duplicate.report_scroll(TimelineScroll::Bottom).await;
    first_room.send_text("independent scroll").await.unwrap();
    first
        .until("away window receives", |item| {
            text(item, "independent scroll")
        })
        .await;
    duplicate
        .until("bottom window receives", |item| {
            text(item, "independent scroll")
        })
        .await;
    assert!(matches!(first.viewport(), TimelineViewport::Away { .. }));
    assert_eq!(duplicate.viewport(), TimelineViewport::Bottom);
    forward_between_windows(
        &phone,
        first_id.as_str(),
        second_id.as_str(),
        &mut first,
        &mut second,
    )
    .await;
    independent_panels(
        &phone,
        first_id.as_str(),
        second_id.as_str(),
        &mut first,
        &mut second,
        "first-open",
    )
    .await;
    duplicate.cancel();
    assert!(duplicate.is_closed().await);
    first_room
        .send_text("after duplicate closes")
        .await
        .unwrap();
    first
        .until("remaining window receives", |item| {
            text(item, "after duplicate closes")
        })
        .await;
    second_room
        .send_text("other room still live")
        .await
        .unwrap();
    second
        .until("other room not revoked", |item| {
            text(item, "other room still live")
        })
        .await;
    first.cancel();
    second.cancel();
    assert!(first.is_closed().await);
    assert!(second.is_closed().await);
    let mut reopened = Timeline::open(&phone, first_id.as_str()).await;
    first_room.send_text("fresh subscription").await.unwrap();
    reopened
        .until("reopened room receives", |item| {
            text(item, "fresh subscription")
        })
        .await;
    let mut reopened_second = Timeline::open(&phone, second_id.as_str()).await;
    independent_panels(
        &phone,
        first_id.as_str(),
        second_id.as_str(),
        &mut reopened,
        &mut reopened_second,
        "reopened",
    )
    .await;
}

async fn forward_between_windows(
    phone: &Phone,
    first_id: &str,
    second_id: &str,
    first: &mut Timeline,
    second: &mut Timeline,
) {
    let (item_id, _) = first
        .until("first window menu", |item| {
            super::message_actions::text_message(item, "first live")
        })
        .await;
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::ForwardMessage {
                target: TimelineTarget {
                    room_id: first_id.to_owned(),
                    thread_root: None,
                },
                item_id,
                to_room_id: second_id.to_owned(),
            },
        })
        .unwrap();
    second
        .until("menu from first window still forwards", |item| {
            text(item, "first live")
        })
        .await;
}

async fn independent_panels(
    phone: &Phone,
    first_id: &str,
    second_id: &str,
    first: &mut Timeline,
    second: &mut Timeline,
    phase: &str,
) {
    let first_tab = wonton_protocol::TabRef::Url {
        url: format!("https://example.com/{phase}/first"),
    };
    let second_tab = wonton_protocol::TabRef::Url {
        url: format!("https://example.com/{phase}/second"),
    };
    for (room_id, tab) in [(first_id, &first_tab), (second_id, &second_tab)] {
        phone
            .dispatch(Action::AddPanelPage {
                scope: PanelScope::Room {
                    room_id: room_id.to_owned(),
                },
                tab: tab.clone(),
            })
            .unwrap();
    }
    for (timeline, wanted, other) in [
        (first, first_tab.clone(), second_tab.clone()),
        (second, second_tab, first_tab),
    ] {
        tokio::time::timeout(super::support::WAIT, async {
            loop {
                let update = timeline.next().await;
                assert!(!update.panel.iter().any(|item| item.tab == other));
                if update.panel.iter().any(|item| item.tab == wanted) {
                    break;
                }
            }
        })
        .await
        .expect("each room receives only its own panel");
    }
}
