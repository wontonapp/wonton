//! 行为：给 ViewModel 的变化序列，看快照文件里写了什么、什么时候写。

use std::time::Duration;

use tokio::sync::watch;
use wonton_protocol::{
    AppearanceViewModel, BotActivity, BotRef, BotView, ChatAccount, ChatFailure, ChatSync,
    ChatViewModel, ConnectionDetails, DeviceVerification, EncryptionStatus, HomeFailure,
    HomeStatus, HomeViewModel, InstalledPlugin, InstalledPlugins, MessageSummary,
    OnboardingViewModel, PluginKind, PluginState, PluginTrust, PluginsViewModel, PushStatus,
    RecoveryNextStep, RecoveryStatus, RoomMembership, RoomPreview, RoomRow, WidgetAccount,
    WidgetPlugin, WidgetSnapshot, WidgetSnapshotRead,
};

use super::{Inputs, MIN_INTERVAL, WidgetFile, read_widget_snapshot, run};
use crate::appearance::default_appearance;

const AGENT: &str = "@hermes:home.example";
const FRIEND: &str = "@alice:home.example";

struct Phone {
    dir: tempfile::TempDir,
    file: WidgetFile,
    written: watch::Receiver<Option<WidgetSnapshot>>,
    home: watch::Sender<HomeViewModel>,
    chat: watch::Sender<ChatViewModel>,
    onboarding: watch::Sender<OnboardingViewModel>,
    plugins: watch::Sender<PluginsViewModel>,
    appearance: watch::Sender<AppearanceViewModel>,
}

impl Phone {
    fn new(home: HomeViewModel, chat: ChatViewModel) -> Self {
        let dir = tempfile::tempdir().unwrap();
        Self::in_dir(dir, home, chat)
    }

    fn in_dir(dir: tempfile::TempDir, home: HomeViewModel, chat: ChatViewModel) -> Self {
        let file = WidgetFile::new(dir.path());
        let mut written = file.subscribe();
        written.mark_unchanged();
        Self {
            dir,
            file,
            written,
            home: watch::Sender::new(home),
            chat: watch::Sender::new(chat),
            onboarding: watch::Sender::new(OnboardingViewModel {
                primary_bot_user_id: Some(AGENT.into()),
                ..OnboardingViewModel::default()
            }),
            plugins: watch::Sender::new(PluginsViewModel::default()),
            appearance: watch::Sender::new(default_appearance()),
        }
    }

    fn start(&self) {
        let inputs = Inputs {
            home: self.home.subscribe(),
            chat: self.chat.subscribe(),
            onboarding: self.onboarding.subscribe(),
            plugins: self.plugins.subscribe(),
            appearance: self.appearance.subscribe(),
            external: false,
        };
        tokio::spawn(run(self.file.clone(), inputs));
    }

    /// 在 `within` 之内写下的快照；没写时为空。
    async fn write_within(&mut self, within: Duration) -> Option<WidgetSnapshot> {
        tokio::time::timeout(within, self.written.changed())
            .await
            .ok()?
            .ok()?;
        self.written.borrow_and_update().clone()
    }

    async fn written_now(&mut self) -> WidgetSnapshot {
        self.write_within(Duration::from_millis(10))
            .await
            .expect("the snapshot should be written right away")
    }

    fn on_disk(&self) -> WidgetSnapshotRead {
        read_widget_snapshot(self.dir.path().to_string_lossy().into_owned())
    }
}

fn paired() -> HomeViewModel {
    HomeViewModel {
        device_id: "ab".repeat(32),
        server_id: Some("cd".repeat(32)),
        status: HomeStatus::Connected,
        path: None,
        notice: None,
        details: ConnectionDetails::default(),
        proxy: None,
        web_url: None,
        pairing: None,
        invite: None,
        keep_connection: false,
        just_recovered: false,
    }
}

fn chat(account: ChatAccount, sync: ChatSync, rooms: Vec<RoomRow>) -> ChatViewModel {
    ChatViewModel {
        chat_layout_default: Default::default(),
        account,
        sync,
        encryption: EncryptionStatus {
            verification: DeviceVerification::Unknown,
            recovery: RecoveryStatus::Unknown,
            recovery_key: None,
            backup_reminder: None,
            can_verify_with_other_device: false,
            can_reset: false,
            recovery_next_step: RecoveryNextStep::None,
            verification_flow: None,
            key_handoff: None,
        },
        installing_bots: Vec::new(),
        rooms,
        problem: None,
        push: PushStatus::NoEndpoint,
        pending_confirmations: Vec::new(),
        settled_confirmations: Vec::new(),
        background_notifications: Vec::new(),
        bots: vec![agent(Some(BotActivity::Waiting))],
    }
}

fn signed_in(rooms: Vec<RoomRow>) -> ChatViewModel {
    chat(
        ChatAccount::SignedIn {
            user_id: "@owner:home.example".into(),
            device_id: "PHONE".into(),
        },
        ChatSync::Running,
        rooms,
    )
}

fn agent(activity: Option<BotActivity>) -> BotView {
    BotView {
        chat: Default::default(),
        user_id: AGENT.into(),
        bot: BotRef {
            plugin_id: "hermes".into(),
            bot: "default".into(),
        },
        web: None,
        name: "Hermes".into(),
        avatar: None,
        state_avatars: Vec::new(),
        activity,
        saving: false,
        problem: None,
    }
}

fn room(name: &str, peer: Option<&str>, unread: u64) -> RoomRow {
    RoomRow {
        layout: Default::default(),
        room_id: format!("!{name}:home.example"),
        name: name.to_owned(),
        is_direct: peer.is_some(),
        direct_user_id: peer.map(str::to_owned),
        is_encrypted: true,
        is_read_only: false,
        membership: RoomMembership::Joined,
        preview: Some(RoomPreview {
            sender_name: name.to_owned(),
            is_own: false,
            body: MessageSummary::Text {
                text: format!("hello from {name}"),
            },
            timestamp_ms: 1,
        }),
        side_chats: false,
        unread_count: unread,
    }
}

fn plugin(id: &str, web: bool) -> InstalledPlugin {
    InstalledPlugin {
        chat: Default::default(),
        id: id.to_owned(),
        title: id.to_uppercase(),
        summary: String::new(),
        version: "1".to_owned(),
        revision: "1".to_owned(),
        kind: PluginKind::App,
        runtime: Default::default(),
        icon: None,
        trust: PluginTrust::Official,
        source: None,
        provider: None,
        via_index: None,
        written_by: None,
        declares_tools: false,
        status: PluginState::Stopped,
        web_url: web.then(|| format!("http://{id}.home.example")),
        cross_origin_isolated: false,
        tabs: Vec::new(),
        bot_user_id: None,
        family: false,
        upgrade: None,
        removable: true,
        notices: Vec::new(),
        enabled_root_permissions: Vec::new(),
        install_progress: None,
        tunnel_ports: Vec::new(),
    }
}

fn installed(plugins: Vec<InstalledPlugin>) -> PluginsViewModel {
    PluginsViewModel {
        installed: Some(InstalledPlugins {
            plugins,
            upgrade_check: None,
        }),
        ..PluginsViewModel::default()
    }
}

/// 十个会话：和 Agent、好友的私聊，八个群，外加一个退出了的和只读的通知私聊。
fn many_rooms() -> Vec<RoomRow> {
    let mut rooms = vec![
        room("hermes", Some(AGENT), 3),
        room("alice", Some(FRIEND), 1),
    ];
    let mut left = room("old", None, 7);
    left.membership = RoomMembership::Left;
    rooms.push(left);
    let mut notices = room("wonton", Some("@wonton-core:home.example"), 0);
    notices.is_read_only = true;
    rooms.push(notices);
    rooms.extend((0..8).map(|n| room(&format!("group{n}"), None, 1)));
    rooms
}

#[tokio::test(start_paused = true)]
async fn a_signed_in_phone_writes_recent_rooms_the_agent_and_search_entries() {
    let mut phone = Phone::new(paired(), signed_in(many_rooms()));
    phone.plugins.send_replace(installed(vec![
        plugin("immich", true),
        plugin("backup", false),
    ]));
    phone.start();

    let snapshot = phone.written_now().await;
    assert_eq!(snapshot.account, WidgetAccount::SignedIn);
    let names: Vec<&str> = snapshot
        .rooms
        .iter()
        .map(|room| room.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "hermes", "alice", "wonton", "group0", "group1", "group2", "group3", "group4"
        ],
        "most recent first, at most eight, the left room is gone"
    );
    assert_eq!(
        snapshot.unread_total,
        3 + 1 + 8,
        "every room counts, not just the eight"
    );
    let agent = snapshot.default_agent.as_ref().unwrap();
    assert_eq!(
        (agent.name.as_str(), agent.activity, agent.unread_count),
        ("Hermes", Some(BotActivity::Waiting), 3)
    );
    let contacts: Vec<(&str, bool)> = snapshot
        .contacts
        .iter()
        .map(|contact| (contact.user_id.as_str(), contact.is_bot))
        .collect();
    assert_eq!(contacts, [(AGENT, true), (FRIEND, false)]);
    assert_eq!(
        snapshot.plugins,
        [WidgetPlugin {
            plugin_id: "immich".into(),
            title: "IMMICH".into(),
        }],
        "only plugins with a page can be opened from search"
    );
    assert_eq!(
        phone.on_disk(),
        WidgetSnapshotRead::Ready { snapshot },
        "what the shell is told is what the widget reads"
    );
}

#[tokio::test(start_paused = true)]
async fn changes_in_quick_succession_are_written_once_two_seconds_later() {
    let mut phone = Phone::new(paired(), signed_in(vec![room("hermes", Some(AGENT), 0)]));
    phone.start();
    phone.written_now().await;

    phone
        .chat
        .send_replace(signed_in(vec![room("hermes", Some(AGENT), 1)]));
    phone
        .chat
        .send_replace(signed_in(vec![room("hermes", Some(AGENT), 2)]));
    let early = phone
        .write_within(MIN_INTERVAL - Duration::from_millis(100))
        .await;
    assert_eq!(
        early, None,
        "nothing is written within two seconds of the last write"
    );

    let later = phone
        .write_within(Duration::from_millis(200))
        .await
        .unwrap();
    assert_eq!(later.unread_total, 2, "the latest state is the one written");
    assert_eq!(
        phone.write_within(Duration::from_secs(10)).await,
        None,
        "the merged changes are written once"
    );
}

#[tokio::test(start_paused = true)]
async fn a_revoked_device_is_cleared_without_waiting() {
    let mut phone = Phone::new(paired(), signed_in(many_rooms()));
    phone.start();
    phone.written_now().await;

    phone.chat.send_replace(chat(
        ChatAccount::Failed {
            reason: ChatFailure::Unauthorized,
            message: "revoked".into(),
        },
        ChatSync::Idle,
        many_rooms(),
    ));
    let cleared = phone.written_now().await;
    assert_eq!(cleared.account, WidgetAccount::SignedOut);
    assert!(cleared.rooms.is_empty() && cleared.contacts.is_empty() && cleared.plugins.is_empty());
    assert_eq!((cleared.unread_total, cleared.default_agent), (0, None));
}

#[tokio::test(start_paused = true)]
async fn a_device_the_home_server_rejects_is_cleared_too() {
    let mut phone = Phone::new(paired(), signed_in(many_rooms()));
    phone.start();
    phone.written_now().await;

    phone.home.send_modify(|home| {
        home.status = HomeStatus::Disconnected {
            reason: HomeFailure::Unauthorized,
            message: "unknown device".into(),
        };
    });
    assert_eq!(phone.written_now().await.account, WidgetAccount::SignedOut);
}

#[tokio::test(start_paused = true)]
async fn while_reconnecting_the_last_snapshot_stays() {
    let mut phone = Phone::new(paired(), signed_in(many_rooms()));
    phone.start();
    let before = phone.written_now().await;

    phone.chat.send_replace(chat(
        ChatAccount::Reconnecting {
            reason: ChatFailure::TransportUnavailable,
            message: "offline".into(),
        },
        ChatSync::Idle,
        many_rooms(),
    ));
    assert_eq!(phone.write_within(Duration::from_secs(60)).await, None);
    assert_eq!(
        phone.on_disk(),
        WidgetSnapshotRead::Ready { snapshot: before }
    );
}

#[tokio::test(start_paused = true)]
async fn a_restored_session_without_rooms_yet_is_not_written_as_empty() {
    let mut restoring = signed_in(Vec::new());
    restoring.sync = ChatSync::Starting;
    let mut phone = Phone::new(paired(), restoring);
    phone.start();
    assert_eq!(phone.write_within(Duration::from_secs(60)).await, None);
    assert_eq!(phone.on_disk(), WidgetSnapshotRead::Missing);

    phone
        .chat
        .send_replace(signed_in(vec![room("hermes", Some(AGENT), 0)]));
    assert_eq!(phone.written_now().await.rooms.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn an_unpaired_phone_writes_that_it_is_not_paired() {
    let mut home = paired();
    home.server_id = None;
    home.status = HomeStatus::NotConfigured;
    let mut phone = Phone::new(
        home,
        chat(ChatAccount::SignedOut, ChatSync::Idle, Vec::new()),
    );
    phone.start();
    assert_eq!(phone.written_now().await.account, WidgetAccount::NotPaired);
}

#[tokio::test(start_paused = true)]
async fn plugins_not_listed_yet_keep_the_ones_from_last_time() {
    let mut first = Phone::new(paired(), signed_in(many_rooms()));
    first
        .plugins
        .send_replace(installed(vec![plugin("immich", true)]));
    first.start();
    first.written_now().await;

    // App 重启：同一个数据目录，插件列表还没从家里取回来。
    let mut second = Phone::in_dir(first.dir, paired(), signed_in(many_rooms()));
    second.start();
    let snapshot = second.written_now().await;
    assert_eq!(snapshot.plugins.len(), 1);
    assert_eq!(snapshot.plugins[0].plugin_id, "immich");
}

#[tokio::test(start_paused = true)]
async fn switching_servers_clears_the_last_one_at_once() {
    let mut phone = Phone::new(paired(), signed_in(many_rooms()));
    phone.start();
    phone.written_now().await;

    phone.file.switching(&default_appearance());
    let cleared = phone.written_now().await;
    assert_eq!(cleared.account, WidgetAccount::Switching);
    assert!(cleared.rooms.is_empty() && cleared.contacts.is_empty());
    assert_eq!(
        phone.on_disk(),
        WidgetSnapshotRead::Ready { snapshot: cleared }
    );
}
