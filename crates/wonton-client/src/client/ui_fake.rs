//! UI 测试用的假数据入口：把 App 直接开进主界面，不需要家里的服务器。
//!
//! 整个模块在 cargo feature `ui-fake` 后面，生产 Release 包不带（构建脚本按 profile 开）；Swift 侧的调用
//! 另在 `#if DEBUG` 里。开着时 `new_ui_fake` 仍然建一个正常的 `ClientCore`（没有后端），再把塞进订阅的
//! ViewModel 换成写死的值；开着时 `dispatch` 不落到作用域，免得网络链路把 ViewModel 改回空的。
//!
//! 只够渲染首屏：一台“家里”的服务器、一个 Bot 私聊、引导已经走完。时间线、媒体和操作后的新状态不在里面，
//! 要用这些的测试仍然连开发栈（见 `apps/ios/WontonUITests/UITestBackend.swift`）。

use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::watch;

use wonton_protocol::{
    ChatAccount, ChatLayout, ChatLayoutChoice, ChatSync, ChatViewModel, CoreError, DeviceNetwork,
    DeviceVerification, EncryptionStatus, HomePath, HomeStatus, HomeViewModel, MessageSummary,
    OnboardingStep, OnboardingViewModel, PushStatus, RecoveryNextStep, RecoveryStatus,
    RoomMembership, RoomPreview, RoomRow, ServerKind, ServerName, ServerRow, ServersViewModel,
    TabItem, TabRef, TabsViewModel,
};

use super::ClientCore;

const ROOM_ID: &str = "!ui-fake:home.example";

#[uniffi::export]
impl ClientCore {
    /// UI 测试专用的启动入口：首屏直接是主界面，内容由 `seed_ui_fake` 写死。壳只在 Debug 里调。
    #[uniffi::constructor]
    pub fn new_ui_fake(data_dir: String) -> Result<Arc<Self>, CoreError> {
        let core = Self::new(data_dir)?;
        core.ui_fake.store(true, Ordering::SeqCst);
        core.seed_ui_fake();
        Ok(core)
    }
}

impl ClientCore {
    fn seed_ui_fake(&self) {
        // 换成独立的一组发送端：真实作用域读盘算完的首帧还会发出来，接在同一组通道上就会把假数据盖回去。
        let current = self.views();
        let views = crate::scope::Views {
            home: watch::Sender::new(fake_home(current.home.borrow().clone())),
            chat: watch::Sender::new(fake_chat(current.chat.borrow().clone())),
            plugins: watch::Sender::new(current.plugins.borrow().clone()),
            tabs: watch::Sender::new(fake_tabs(current.tabs.borrow().clone())),
            transfers: watch::Sender::new(current.transfers.borrow().clone()),
            onboarding: watch::Sender::new(fake_onboarding(current.onboarding.borrow().clone())),
            peers: watch::Sender::new(current.peers.borrow().clone()),
            friends: watch::Sender::new(current.friends.borrow().clone()),
            bot_contacts: watch::Sender::new(current.bot_contacts.borrow().clone()),
            settings: watch::Sender::new(current.settings.borrow().clone()),
        };
        drop(current);
        self.servers.seed_ui_fake(fake_servers());
        *self
            .views
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = views;
    }
}

fn fake_home(mut home: HomeViewModel) -> HomeViewModel {
    home.server_id = Some("ui-fake".into());
    home.status = HomeStatus::Connected;
    home.path = Some(HomePath {
        relay: false,
        public_relay: false,
        lan: true,
        rtt_ms: 12,
    });
    home.details.network = Some(DeviceNetwork::Wifi);
    home.details.relay = Default::default();
    home
}

fn fake_servers() -> ServersViewModel {
    ServersViewModel {
        servers: vec![ServerRow {
            id: "ui-fake".into(),
            name: ServerName::Named {
                name: "Home".into(),
            },
            kind: ServerKind::Wonton,
            current: true,
        }],
        adding: false,
        matrix_login: None,
    }
}

fn fake_chat(mut chat: ChatViewModel) -> ChatViewModel {
    chat.account = ChatAccount::SignedIn {
        user_id: "@owner:home.example".into(),
        device_id: "UIFAKE".into(),
    };
    chat.sync = ChatSync::Running;
    chat.encryption = EncryptionStatus {
        verification: DeviceVerification::Unverified,
        recovery: RecoveryStatus::Incomplete,
        recovery_key: None,
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
    chat.rooms = vec![RoomRow {
        room_id: ROOM_ID.into(),
        name: "Echo".into(),
        is_direct: true,
        direct_user_id: Some("@echo:home.example".into()),
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
        side_chats: false,
        layout: ChatLayout::default(),
    }];
    chat.problem = None;
    chat.push = PushStatus::NoEndpoint;
    chat.pending_confirmations.clear();
    chat.settled_confirmations.clear();
    chat.background_notifications.clear();
    chat.bots.clear();
    chat.chat_layout_default = ChatLayoutChoice::default();
    chat
}

fn fake_tabs(mut tabs: TabsViewModel) -> TabsViewModel {
    tabs.items = vec![
        TabItem {
            tab: TabRef::Chat,
            removable: false,
            badge: false,
            page: None,
        },
        TabItem {
            tab: TabRef::Apps,
            removable: true,
            badge: false,
            page: None,
        },
        TabItem {
            tab: TabRef::Settings,
            removable: true,
            badge: false,
            page: None,
        },
    ];
    tabs.show_bar = true;
    tabs.selected = TabRef::Chat;
    tabs.overlay = None;
    tabs
}

fn fake_onboarding(mut onboarding: OnboardingViewModel) -> OnboardingViewModel {
    onboarding.step = Some(OnboardingStep::Finished);
    onboarding.nav = None;
    onboarding.loading = false;
    onboarding.problem = None;
    onboarding
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 假数据塞进了各订阅、操作不落到作用域：漏填字段或写坏类型在这里就炸。
    #[test]
    fn seeded_views_are_set_and_dispatch_is_inert() {
        let dir = std::env::temp_dir().join(format!("wonton-ui-fake-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let core = ClientCore::new_ui_fake(dir.to_string_lossy().into_owned()).unwrap();
        assert!(core.ui_fake.load(Ordering::SeqCst));
        assert_eq!(core.home().status, HomeStatus::Connected);
        assert_eq!(core.servers().servers.len(), 1);
        assert_eq!(core.chat().rooms[0].room_id, ROOM_ID);
        assert_eq!(core.tabs().selected, TabRef::Chat);
        assert_eq!(core.onboarding().step, Some(OnboardingStep::Finished));
        // 作用域的首帧是后台算完才发的：等一会儿，确认它盖不掉假数据。
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert_eq!(core.home().status, HomeStatus::Connected);
        assert_eq!(core.chat().rooms[0].name, "Echo");
        assert_eq!(core.tabs().selected, TabRef::Chat);
        assert_eq!(core.onboarding().step, Some(OnboardingStep::Finished));
        // 操作不落到作用域。
        assert!(
            core.dispatch(wonton_protocol::Action::SelectTab {
                tab: TabRef::Settings
            })
            .is_ok()
        );
        assert_eq!(core.tabs().selected, TabRef::Chat);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
