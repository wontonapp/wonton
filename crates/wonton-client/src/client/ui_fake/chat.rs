//! 假房间和假聊天列表：私聊、通话中的群、没人说话只给转发用的群，以及 Bot 私聊的 ViewModel。
//!
//! 房间 ID 和假群聊输入框的 @ 候选也在这里；`ROOM_ID`、`CALL_ROOM_ID`、`mention_candidates` 由
//! `ui_fake.rs` 再导出给 `client` 下的其他模块。

use wonton_protocol::{
    BotActivity, BotChatAvailability, BotRef, BotView, BotWebView, ChatAccount, ChatLayout,
    ChatLayoutChoice, ChatSync, ChatViewModel, DeviceVerification, EncryptionStatus,
    MessageSummary, PluginState, PushStatus, RecoveryNextStep, RecoveryStatus, RoomMembership,
    RoomPlugin, RoomPreview, RoomRow, SecretString,
};

use super::tabs::fake_bot_page;

pub(in crate::client) const ROOM_ID: &str = "!ui-fake:home.example";
/// 有人在通话的群聊：列表这一行和顶栏都标“通话中”，顶栏有 [`CALL_PLUGIN`](super::tabs::CALL_PLUGIN) 的 widget 入口。
pub(in crate::client) const CALL_ROOM_ID: &str = "!ui-fake-call:home.example";
/// 没人说话的群，转发时多一个去处。
const NOTES_ROOM_ID: &str = "!ui-fake-notes:home.example";

/// 假聊天列表：私聊、通话中的群、没人说话只给转发用的群。
pub(super) fn fake_rooms() -> Vec<RoomRow> {
    vec![
        RoomRow {
            unread_mentions: 0,
            room_id: ROOM_ID.into(),
            name: "Echo".into(),
            is_direct: true,
            direct_user_id: Some("@echo:home.example".into()),
            plugin: None,
            bot_chat: None,
            bot: None,
            is_encrypted: true,
            is_read_only: false,
            membership: RoomMembership::Joined,
            preview: Some(RoomPreview {
                sender_name: "Echo".into(),
                is_own: false,
                body: MessageSummary::Text {
                    text: "Fake message from the UI test fixture".into(),
                },
                timestamp_ms: 1_792_000_000_000,
            }),
            unread_count: 1,
            // 旁聊卡片挂在这条私聊里：界面测试从卡片进旁聊。
            side_chats: true,
            layout: ChatLayout::default(),
            call_active: false,
        },
        RoomRow {
            unread_mentions: 0,
            room_id: CALL_ROOM_ID.into(),
            name: "Family".into(),
            is_direct: false,
            direct_user_id: None,
            plugin: None,
            bot_chat: None,
            bot: None,
            is_encrypted: true,
            is_read_only: false,
            membership: RoomMembership::Joined,
            preview: None,
            unread_count: 0,
            side_chats: false,
            layout: ChatLayout::default(),
            call_active: true,
        },
        // 转发要能一次挑好几个房间，列表里多放一个没人说话的群；顶栏下插件横幅、旁聊标题、AI 行一起出现，
        // 界面测试在这间房里看它们的上下顺序。
        RoomRow {
            unread_mentions: 0,
            room_id: NOTES_ROOM_ID.into(),
            name: "Notes".into(),
            is_direct: false,
            direct_user_id: None,
            plugin: Some(RoomPlugin {
                plugin_id: "app.echo".into(),
                status: PluginState::Installing,
                start: None,
                can_start: false,
            }),
            bot_chat: None,
            bot: None,
            is_encrypted: true,
            is_read_only: false,
            membership: RoomMembership::Joined,
            preview: None,
            unread_count: 0,
            side_chats: true,
            layout: ChatLayout::default(),
            call_active: false,
        },
    ]
}

pub(super) fn fake_chat(mut chat: ChatViewModel) -> ChatViewModel {
    chat.account = ChatAccount::SignedIn {
        user_id: "@owner:home.example".into(),
        device_id: "UIFAKE".into(),
    };
    chat.sync = ChatSync::Running;
    chat.encryption = EncryptionStatus {
        verification: DeviceVerification::Unverified,
        recovery: RecoveryStatus::Incomplete,
        // 界面测试要一把刚生成、还没抄下的恢复密钥（复制按钮的反馈和剪贴板）时给 `WONTON_UI_FAKE_RECOVERY_KEY`。
        recovery_key: std::env::var("WONTON_UI_FAKE_RECOVERY_KEY")
            .ok()
            .map(SecretString::new),
        backup_reminder: None,
        can_verify_with_other_device: false,
        can_reset: false,
        recovery_next_step: RecoveryNextStep::None,
        verification_flow: None,
        key_handoff: None,
    };
    // 和真核心同一份推导，备份页在假核心下也按真值表显示。
    crate::chat::recovery::refresh(&mut chat.encryption);
    chat.installing_bots.clear();
    chat.rooms = fake_rooms();
    chat.problem = None;
    chat.push = PushStatus::NoEndpoint;
    chat.pending_confirmations.clear();
    chat.settled_confirmations.clear();
    chat.background_notifications.clear();
    // 两个 Bot：私聊那个顶栏本来就显示，群里的那个走时间线的 room_bots（ui_fake_room）。
    chat.bots = vec![
        BotView {
            user_id: "@echo:home.example".into(),
            availability: BotChatAvailability::Available,
            bot: BotRef {
                plugin_id: "app.example.echo".into(),
                bot: "echo".into(),
            },
            web: Some(BotWebView::Ready {
                page: fake_bot_page("app.example.echo", "echo"),
            }),
            web_page: Some(fake_bot_page("app.example.echo", "echo")),
            name: "Echo".into(),
            avatar: None,
            state_avatars: Vec::new(),
            activity: Some(BotActivity::Idle),
            saving: false,
            problem: None,
            chat: Default::default(),
        },
        BotView {
            user_id: "@hermes:home.example".into(),
            availability: BotChatAvailability::Available,
            bot: BotRef {
                plugin_id: "app.example.hermes".into(),
                bot: "hermes".into(),
            },
            web: None,
            web_page: None,
            name: "Hermes".into(),
            avatar: None,
            state_avatars: Vec::new(),
            activity: Some(BotActivity::Idle),
            saving: false,
            problem: None,
            chat: Default::default(),
        },
    ];
    chat.chat_layout_default = ChatLayoutChoice::default();
    chat
}

/// 假群聊输入框的 @ 候选：一个 Bot、一个人；私聊里不给。
pub(in crate::client) fn mention_candidates(
    room_id: &str,
) -> Vec<wonton_protocol::ComposerMention> {
    if room_id != CALL_ROOM_ID {
        return Vec::new();
    }
    [
        ("@hermes:home.example", "Hermes", true),
        ("@tom:home.example", "Tom", false),
    ]
    .into_iter()
    .map(
        |(user_id, display_name, is_bot)| wonton_protocol::ComposerMention {
            user_id: user_id.to_owned(),
            display_name: display_name.to_owned(),
            avatar: None,
            is_bot,
        },
    )
    .collect()
}
