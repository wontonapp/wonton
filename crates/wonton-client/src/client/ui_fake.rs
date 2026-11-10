//! UI 测试用的假数据入口：把 App 直接开进主界面，不需要家里的服务器。
//!
//! 假数据不进生产路径：`new_ui_fake` 仍然建一个正常的 `ClientCore`（没有后端），再把塞进订阅的 ViewModel 换成
//! 写死的值；开着时 `dispatch` 不落到作用域，免得网络链路把 ViewModel 改回空的。壳只在 `#if DEBUG` 里调它，
//! 入口跟着绑定一起编进包里，Release 构建也能生成同一份 Swift 绑定。
//!
//! 只够渲染首屏：一台“家里”的服务器、一个 Bot 私聊、引导已经走完。时间线、媒体和操作后的新状态不在里面，
//! 要用这些的测试仍然连开发栈（见 `apps/ios/WontonUITests/UITestBackend.swift`）。

use std::sync::Arc;
use std::sync::atomic::Ordering;

use wonton_protocol::{
    ChatAccount, ChatLayout, ChatLayoutChoice, ChatSync, CoreError, DeviceNetwork,
    DeviceVerification, EncryptionStatus, HomePath, HomeStatus, MessageSummary, OnboardingStep,
    PushStatus, RecoveryStatus, RoomMembership, RoomPreview, RoomRow, ServerKind, ServerName,
    ServerRow, ServersViewModel, TabItem, TabRef,
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
        let views = &self.views;
        let mut home = self.home();
        home.server_id = Some("ui-fake".into());
        home.status = HomeStatus::Connected;
        home.path = Some(HomePath {
            relay: false,
            lan: true,
            rtt_ms: 12,
        });
        home.details.network = Some(DeviceNetwork::Wifi);
        home.details.relay = Default::default();
        views.home.send_replace(home);

        self.servers.seed_ui_fake(ServersViewModel {
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
        });

        let mut chat = self.chat();
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
            verification_flow: None,
        };
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
            layout: ChatLayout::default(),
        }];
        chat.problem = None;
        chat.push = PushStatus::NoEndpoint;
        chat.pending_confirmations.clear();
        chat.settled_confirmations.clear();
        chat.background_notifications.clear();
        chat.bots.clear();
        chat.chat_layout_default = ChatLayoutChoice::default();
        views.chat.send_replace(chat);

        let mut tabs = self.tabs();
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
        views.tabs.send_replace(tabs);

        let mut onboarding = self.onboarding();
        onboarding.step = Some(OnboardingStep::Finished);
        onboarding.nav = None;
        onboarding.loading = false;
        onboarding.problem = None;
        views.onboarding.send_replace(onboarding);
    }
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
