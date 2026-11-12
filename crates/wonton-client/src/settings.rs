//! 设置首页的 ViewModel：只由连接、聊天和插件的 ViewModel 算出来，自己不存东西。

use tokio::runtime::Handle;
use tokio::sync::watch;
use wonton_protocol::{
    ChatViewModel, HomeStatus, HomeViewModel, NotificationCategory, PluginsViewModel, PushStatus,
    RecoveryStatus, SettingsEntry, SettingsSection, SettingsViewModel,
};

pub(crate) struct SettingsLink {
    view: watch::Sender<SettingsViewModel>,
}

impl SettingsLink {
    pub(crate) fn new(
        runtime: &Handle,
        mut home: watch::Receiver<HomeViewModel>,
        mut chat: watch::Receiver<ChatViewModel>,
        mut plugins: watch::Receiver<PluginsViewModel>,
    ) -> Self {
        let compute = |home: &watch::Receiver<HomeViewModel>,
                       chat: &watch::Receiver<ChatViewModel>,
                       plugins: &watch::Receiver<PluginsViewModel>| {
            let chat = chat.borrow();
            settings_view(
                &home.borrow().status,
                chat.encryption.recovery,
                &chat.push,
                plugins.borrow().core_upgrade.is_prompting(),
            )
        };
        let view = watch::Sender::new(compute(&home, &chat, &plugins));
        let sender = view.clone();
        runtime.spawn(async move {
            loop {
                tokio::select! {
                    changed = home.changed() => if changed.is_err() { break },
                    changed = chat.changed() => if changed.is_err() { break },
                    changed = plugins.changed() => if changed.is_err() { break },
                }
                let next = compute(&home, &chat, &plugins);
                sender.send_if_modified(|view| {
                    let modified = *view != next;
                    *view = next;
                    modified
                });
            }
        });
        Self { view }
    }

    pub(crate) fn view(&self) -> SettingsViewModel {
        self.view.borrow().clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<SettingsViewModel> {
        self.view.subscribe()
    }
}

/// 常用的放前面：资料和服务器各占一组，然后是这台手机上的外观、底栏和通知，再是聊天相关的，最后是很少点的。
/// 资料、服务器、默认 Agent 和引导都存在服务器上，没配对时不出现。
fn settings_view(
    status: &HomeStatus,
    recovery: RecoveryStatus,
    push: &PushStatus,
    upgrade: bool,
) -> SettingsViewModel {
    let paired = !matches!(status, HomeStatus::NotConfigured);
    let backup = SettingsEntry::ChatBackup { recovery };
    let phone = vec![
        SettingsEntry::Appearance,
        SettingsEntry::BottomBar,
        SettingsEntry::Notifications {
            push: push.clone(),
            categories: NotificationCategory::ALL.to_vec(),
        },
    ];
    let groups = if paired {
        vec![
            vec![SettingsEntry::Profile],
            vec![SettingsEntry::Server {
                name: None,
                status: status.clone(),
                upgrade,
            }],
            phone,
            vec![SettingsEntry::DefaultAgent, backup],
            vec![SettingsEntry::RestartOnboarding, SettingsEntry::About],
        ]
    } else {
        vec![phone, vec![backup], vec![SettingsEntry::About]]
    };
    SettingsViewModel {
        sections: groups
            .into_iter()
            .map(|entries| SettingsSection { entries })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(view: &SettingsViewModel) -> Vec<SettingsEntry> {
        view.sections
            .iter()
            .flat_map(|section| section.entries.clone())
            .collect()
    }

    #[test]
    fn before_pairing_only_this_phones_settings_show() {
        let view = settings_view(
            &HomeStatus::NotConfigured,
            RecoveryStatus::Unknown,
            &PushStatus::NoEndpoint,
            false,
        );
        assert_eq!(
            entries(&view),
            [
                SettingsEntry::Appearance,
                SettingsEntry::BottomBar,
                SettingsEntry::Notifications {
                    push: PushStatus::NoEndpoint,
                    categories: vec![
                        NotificationCategory::Message,
                        NotificationCategory::Confirmation,
                        NotificationCategory::PluginNotice
                    ],
                },
                SettingsEntry::ChatBackup {
                    recovery: RecoveryStatus::Unknown
                },
                SettingsEntry::About,
            ]
        );
    }

    #[test]
    fn after_pairing_profile_and_server_come_first() {
        let view = settings_view(
            &HomeStatus::Connected,
            RecoveryStatus::Enabled,
            &PushStatus::Registered,
            true,
        );
        assert_eq!(view.sections[0].entries, [SettingsEntry::Profile]);
        assert_eq!(
            view.sections[1].entries,
            [SettingsEntry::Server {
                name: None,
                status: HomeStatus::Connected,
                upgrade: true
            }]
        );
        assert!(entries(&view).contains(&SettingsEntry::DefaultAgent));
        assert!(entries(&view).contains(&SettingsEntry::RestartOnboarding));
    }

    #[tokio::test]
    async fn follows_the_connection_push_and_core_upgrades() {
        let home = |status| HomeViewModel {
            device_id: "device".into(),
            server_id: Some("server".into()),
            status,
            path: None,
            notice: None,
            details: Default::default(),
            proxy: None,
            web_url: None,
            pairing: None,
            invite: None,
        };
        let chat = ChatViewModel {
            account: wonton_protocol::ChatAccount::SignedOut,
            sync: wonton_protocol::ChatSync::Idle,
            encryption: wonton_protocol::EncryptionStatus {
                verification: wonton_protocol::DeviceVerification::Unknown,
                recovery: RecoveryStatus::Unknown,
                recovery_key: None,
                backup_reminder: None,
            },
            rooms: Vec::new(),
            problem: None,
            push: PushStatus::NoEndpoint,
            pending_confirmations: Vec::new(),
            settled_confirmations: Vec::new(),
            bots: Vec::new(),
        };
        let (home_tx, home_rx) = watch::channel(home(HomeStatus::NotConfigured));
        let (chat_tx, chat_rx) = watch::channel(chat);
        let (plugins_tx, plugins_rx) = watch::channel(PluginsViewModel::default());
        let link = SettingsLink::new(&Handle::current(), home_rx, chat_rx, plugins_rx);
        let mut updates = link.subscribe();
        assert!(!entries(&link.view()).contains(&SettingsEntry::Profile));

        home_tx.send_replace(home(HomeStatus::Connected));
        updates.changed().await.unwrap();
        assert!(entries(&link.view()).contains(&SettingsEntry::Profile));

        chat_tx.send_modify(|chat| chat.push = PushStatus::Registered);
        updates.changed().await.unwrap();
        assert!(entries(&link.view()).iter().any(|entry| matches!(
            entry,
            SettingsEntry::Notifications {
                push: PushStatus::Registered,
                ..
            }
        )));

        // 跳过的版本不提示，和升级页的“跳过”一致。
        plugins_tx.send_modify(|plugins| {
            plugins.core_upgrade.offer = Some(wonton_protocol::CoreUpgradeOffer {
                digest: "sha256:new".into(),
                version: Some("0.2.0".into()),
                migrates: Vec::new(),
            });
        });
        updates.changed().await.unwrap();
        let server = |link: &SettingsLink| link.view().sections[1].entries[0].clone();
        assert_eq!(
            server(&link),
            SettingsEntry::Server {
                name: None,
                status: HomeStatus::Connected,
                upgrade: true
            }
        );
        plugins_tx.send_modify(|plugins| plugins.core_upgrade.offer_skipped = true);
        updates.changed().await.unwrap();
        assert_eq!(
            server(&link),
            SettingsEntry::Server {
                name: None,
                status: HomeStatus::Connected,
                upgrade: false
            }
        );
    }
}
