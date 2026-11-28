//! 没订阅时间线时给用户看的会话叫法：有名字用名字，没名字用对方名或会话类型，别露房间 ID。

use super::*;
use crate::l10n::{CATALOG, language};
use wonton_l10n::l10n;
use wonton_protocol::{BotRef, ChatLayout, RoomMembership};

fn key(room: &str) -> TimelineKey {
    TimelineKey::main(room.parse().unwrap())
}

fn row(room: &str, name: &str, peer: Option<&str>) -> RoomRow {
    RoomRow {
        unread_mentions: 0,
        room_id: room.to_owned(),
        name: name.to_owned(),
        is_direct: peer.is_some(),
        direct_user_id: peer.map(str::to_owned),
        plugin: None,
        bot_chat: None,
        bot: None,
        is_encrypted: false,
        is_read_only: false,
        membership: RoomMembership::Joined,
        preview: None,
        side_chats: false,
        unread_count: 0,
        layout: ChatLayout::default(),
        call_active: false,
    }
}

fn bot(user: &str, name: &str) -> BotView {
    BotView {
        chat: Default::default(),
        availability: Default::default(),
        user_id: user.to_owned(),
        shared: false,
        danger: None,
        bot: BotRef {
            plugin_id: "a.b".into(),
            bot: "default".into(),
        },
        web: None,
        web_page: None,
        name: name.to_owned(),
        avatar: None,
        cloud: None,
        custom_media: false,
        face: None,
        state_avatars: Vec::new(),
        activity: None,
        saving: false,
        problem: None,
    }
}

fn reason(rooms: &[RoomRow], bots: &[BotView], key: &TimelineKey) -> String {
    match not_subscribed(&conversation_label(rooms, bots, key)) {
        CoreError::NotReady { reason } => reason,
        other => panic!("expected not ready, got {other:?}"),
    }
}

fn expected(conversation: String) -> String {
    l10n!(
        CATALOG,
        language(),
        "chat-room-not-subscribed",
        conversation = conversation
    )
}

#[test]
fn a_named_room_is_pointed_at_by_its_name() {
    let reason = reason(
        &[row("!room:example.com", "家庭群", None)],
        &[],
        &key("!room:example.com"),
    );
    assert!(reason.contains("家庭群"), "{reason}");
    assert!(!reason.contains('!'), "{reason}");
}

#[test]
fn a_room_without_a_name_uses_the_conversation_kind() {
    let reason = reason(
        &[row(
            "!room:example.com",
            "!room:example.com",
            Some("@alice:example.com"),
        )],
        &[],
        &key("!room:example.com"),
    );
    assert_eq!(
        reason,
        expected(l10n!(CATALOG, language(), "chat-conversation-direct"))
    );
    assert!(!reason.contains('!'), "{reason}");
}

#[test]
fn a_direct_chat_falls_back_to_the_bot_name() {
    let reason = reason(
        &[row(
            "!room:example.com",
            "@alice:example.com",
            Some("@alice:example.com"),
        )],
        &[bot("@alice:example.com", "Hermes")],
        &key("!room:example.com"),
    );
    assert!(reason.contains("Hermes"), "{reason}");
    assert!(!reason.contains('!'), "{reason}");
}

#[test]
fn an_unknown_room_is_this_conversation() {
    let reason = reason(&[], &[], &key("!room:example.com"));
    assert_eq!(
        reason,
        expected(l10n!(CATALOG, language(), "chat-conversation-this"))
    );
    assert!(!reason.contains('!'), "{reason}");
}
