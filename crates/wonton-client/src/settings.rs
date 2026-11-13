//! 设置首页的 ViewModel：只由连接、聊天和插件的 ViewModel 算出来，自己不存东西。

use tokio::runtime::Handle;
use tokio::sync::watch;
use wonton_protocol::{
    ChatViewModel, FriendsState, FriendsViewModel, HomeStatus, HomeViewModel, NotificationCategory,
    PeersState, PeersViewModel, PluginsViewModel, PushMethod, PushStatus, RecoveryStatus,
    ServerName, ServersViewModel, SettingsEntry, SettingsSection, SettingsViewModel,
};

use crate::scope::reuse;
use crate::servers::current_name;

pub(crate) struct SettingsLink {
    view: watch::Sender<SettingsViewModel>,
}

impl SettingsLink {
    // 订阅来源就是有这么多：home、chat、plugins、peers、servers、method 加上要通知的 view，收进结构体只是搬家。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        runtime: &Handle,
        mut home: watch::Receiver<HomeViewModel>,
        mut chat: watch::Receiver<ChatViewModel>,
        mut plugins: watch::Receiver<PluginsViewModel>,
        mut peers: watch::Receiver<PeersViewModel>,
        mut friends: watch::Receiver<FriendsViewModel>,
        mut servers: watch::Receiver<ServersViewModel>,
        mut method: watch::Receiver<PushMethod>,
        view: Option<watch::Sender<SettingsViewModel>>,
    ) -> Self {
        let compute = |home: &watch::Receiver<HomeViewModel>,
                       chat: &watch::Receiver<ChatViewModel>,
                       plugins: &watch::Receiver<PluginsViewModel>,
                       peers: &watch::Receiver<PeersViewModel>,
                       friends: &watch::Receiver<FriendsViewModel>,
                       servers: &watch::Receiver<ServersViewModel>,
                       method: &watch::Receiver<PushMethod>| {
            // 同一个 watch 不能同时借两份：发送方在两次 borrow 之间等写锁时，第二次 borrow 排在它后面，
            // 第一份又不会还，两个线程一起卡死。
            let peer_count = {
                let peers = peers.borrow();
                (peers.state == PeersState::Ready).then(|| peers.peers.len())
            };
            let chat = chat.borrow();
            settings_view(
                &home.borrow().status,
                current_name(&servers.borrow()),
                chat.encryption.recovery,
                *method.borrow(),
                &chat.push,
                plugins.borrow().core_upgrade.is_prompting(),
                peer_count,
                friends_count(&friends.borrow()),
            )
        };
        let view = reuse(
            view,
            compute(&home, &chat, &plugins, &peers, &friends, &servers, &method),
        );
        let sender = view.clone();
        runtime.spawn(async move {
            loop {
                tokio::select! {
                    changed = home.changed() => if changed.is_err() { break },
                    changed = chat.changed() => if changed.is_err() { break },
                    changed = plugins.changed() => if changed.is_err() { break },
                    changed = peers.changed() => if changed.is_err() { break },
                    changed = friends.changed() => if changed.is_err() { break },
                    changed = servers.changed() => if changed.is_err() { break },
                    changed = method.changed() => if changed.is_err() { break },
                }
                let next = compute(&home, &chat, &plugins, &peers, &friends, &servers, &method);
                sender.send_if_modified(|view| {
                    let modified = *view != next;
                    *view = next;
                    modified
                });
            }
        });
        Self { view }
    }

    #[cfg(test)]
    fn view(&self) -> SettingsViewModel {
        self.view.borrow().clone()
    }

    /// 换服务器时新作用域接着用这个发送端，壳的订阅不断（ADR-0042）。
    pub(crate) fn sender(&self) -> watch::Sender<SettingsViewModel> {
        self.view.clone()
    }
}

/// 好友页的人数、待处理请求数；还没取到列表时是 0。配对后入口一直可见。
fn friends_count(view: &FriendsViewModel) -> Option<(u32, u32)> {
    let ready = view.state == FriendsState::Ready;
    Some((
        if ready {
            u32::try_from(view.friends.len()).unwrap_or(u32::MAX)
        } else {
            0
        },
        if ready {
            u32::try_from(view.incoming.len()).unwrap_or(u32::MAX)
        } else {
            0
        },
    ))
}

/// 常用的放前面：资料和服务器各占一组，然后是这台手机上的外观、语言、底栏和通知，再是聊天相关的，最后是很少点的。
/// 资料、服务器、默认 Agent 和引导都存在服务器上，没配对时不出现。
/// “其他服务器”只给主人：核心的配对接口只让主人设备读，列表取到过（`Some`）才说明这台设备是主人的，
/// 台数才拿得到。
#[allow(clippy::too_many_arguments)]
fn settings_view(
    status: &HomeStatus,
    name: Option<ServerName>,
    recovery: RecoveryStatus,
    method: PushMethod,
    push: &PushStatus,
    upgrade: bool,
    peers: Option<usize>,
    friends: Option<(u32, u32)>,
) -> SettingsViewModel {
    let paired = !matches!(status, HomeStatus::NotConfigured);
    let backup = SettingsEntry::ChatBackup { recovery };
    let phone = vec![
        SettingsEntry::Appearance,
        SettingsEntry::Language,
        SettingsEntry::MyApps,
        SettingsEntry::Notifications {
            method,
            push: push.clone(),
            categories: NotificationCategory::ALL.to_vec(),
        },
    ];
    let groups = if paired {
        let mut groups = vec![
            vec![SettingsEntry::Profile],
            vec![SettingsEntry::Server {
                name,
                status: status.clone(),
                upgrade,
            }],
            phone,
            vec![SettingsEntry::DefaultAgent, backup],
            vec![SettingsEntry::RestartOnboarding, SettingsEntry::About],
        ];
        // 配对后入口一直可见；还没取到列表时按 0 显示（见 friends_count）。
        let (friends_count, requests) = friends.unwrap_or((0, 0));
        groups[0].push(SettingsEntry::Friends {
            count: friends_count,
            requests,
        });
        if let Some(peers_count) = peers {
            // “其他服务器”和服务器管理是一类：放在服务器的同一组里。
            groups[1].push(SettingsEntry::Peers {
                count: u32::try_from(peers_count).unwrap_or(u32::MAX),
            });
        }
        groups
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
    fn language_comes_right_after_appearance_before_and_after_pairing() {
        for status in [HomeStatus::NotConfigured, HomeStatus::Connected] {
            let view = settings_view(
                &status,
                None,
                RecoveryStatus::Unknown,
                PushMethod::Keepalive,
                &PushStatus::NoEndpoint,
                false,
                Some(0),
                None,
            );
            let all = entries(&view);
            let appearance = all
                .iter()
                .position(|entry| entry == &SettingsEntry::Appearance)
                .expect("the settings home has an appearance entry");
            assert_eq!(all.get(appearance + 1), Some(&SettingsEntry::Language));
        }
    }

    #[test]
    fn before_pairing_only_this_phones_settings_show() {
        let view = settings_view(
            &HomeStatus::NotConfigured,
            None,
            RecoveryStatus::Unknown,
            PushMethod::Keepalive,
            &PushStatus::NoEndpoint,
            false,
            None,
            None,
        );
        assert_eq!(
            entries(&view),
            [
                SettingsEntry::Appearance,
                SettingsEntry::Language,
                SettingsEntry::MyApps,
                SettingsEntry::Notifications {
                    method: PushMethod::Keepalive,
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
            Some(ServerName::Named {
                name: "办公室".into(),
            }),
            RecoveryStatus::Enabled,
            PushMethod::Keepalive,
            &PushStatus::Registered,
            true,
            None,
            None,
        );
        assert_eq!(
            view.sections[0].entries,
            [
                SettingsEntry::Profile,
                SettingsEntry::Friends {
                    count: 0,
                    requests: 0
                }
            ]
        );
        assert_eq!(
            view.sections[1].entries,
            [SettingsEntry::Server {
                name: Some(ServerName::Named {
                    name: "办公室".into(),
                }),
                status: HomeStatus::Connected,
                upgrade: true
            }]
        );
        assert!(entries(&view).contains(&SettingsEntry::DefaultAgent));
        assert!(entries(&view).contains(&SettingsEntry::RestartOnboarding));
    }

    /// 好友入口只在配对后出现，取到列表才带人数和待处理请求数。
    #[test]
    fn the_friends_entry_needs_pairing_and_a_loaded_list() {
        let view = |status: &HomeStatus, friends: Option<(u32, u32)>| {
            settings_view(
                status,
                None,
                RecoveryStatus::Unknown,
                PushMethod::Keepalive,
                &PushStatus::NoEndpoint,
                false,
                None,
                friends,
            )
        };
        assert!(
            !entries(&view(&HomeStatus::NotConfigured, Some((3, 1)))).contains(
                &SettingsEntry::Friends {
                    count: 3,
                    requests: 1
                }
            )
        );
        assert!(
            entries(&view(&HomeStatus::Connected, None))
                .iter()
                .any(|entry| matches!(entry, SettingsEntry::Friends { .. }))
        );
        assert!(
            entries(&view(&HomeStatus::Connected, Some((3, 1)))).contains(
                &SettingsEntry::Friends {
                    count: 3,
                    requests: 1
                }
            )
        );
    }

    /// “其他服务器”跟着配对列表走：列表取到过（这台设备是主人的）才给，台数是已配对的数量。
    #[test]
    fn the_peers_entry_follows_the_peers_list() {
        let view = |peers: Option<usize>| {
            settings_view(
                &HomeStatus::Connected,
                None,
                RecoveryStatus::Unknown,
                PushMethod::Keepalive,
                &PushStatus::NoEndpoint,
                false,
                peers,
                None,
            )
        };
        let server_group = |view: &SettingsViewModel| view.sections[1].entries.clone();
        assert_eq!(
            server_group(&view(None)),
            [SettingsEntry::Server {
                name: None,
                status: HomeStatus::Connected,
                upgrade: false
            }]
        );
        assert_eq!(
            server_group(&view(Some(2))),
            [
                SettingsEntry::Server {
                    name: None,
                    status: HomeStatus::Connected,
                    upgrade: false
                },
                SettingsEntry::Peers { count: 2 },
            ]
        );
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
            keep_connection: false,
            just_recovered: false,
        };
        let (home_tx, home_rx) = watch::channel(home(HomeStatus::NotConfigured));
        let (chat_tx, chat_rx) = watch::channel(chat_view_model());
        let (plugins_tx, plugins_rx) = watch::channel(PluginsViewModel::default());
        let (peers_tx, peers_rx) = watch::channel(peers_view_model());
        let (_friends_tx, friends_rx) = watch::channel(friends_view_model());
        let (_servers_tx, servers_rx) = watch::channel(ServersViewModel::default());
        let (_method_tx, method_rx) = watch::channel(PushMethod::default());
        let link = SettingsLink::new(
            &Handle::current(),
            home_rx,
            chat_rx,
            plugins_rx,
            peers_rx,
            friends_rx,
            servers_rx,
            method_rx,
            None,
        );
        let mut updates = link.sender().subscribe();
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
                compat: wonton_protocol::CoreUpgradeCompat::Unavailable,
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

        // 配对列表取到了，“其他服务器”这一行才出现，台数是已配对的数量。
        peers_tx.send_modify(|peers| {
            peers.state = PeersState::Ready;
            peers.peers = vec![wonton_protocol::PeerRow {
                server_name: "abc12345.wonton".into(),
                owner_user_id: "@alex:def67890.wonton".into(),
                owner_display_name: "Alex".into(),
                status: wonton_protocol::PeerStatus::Connected,
                last_connected_ms: None,
                detail: None,
            }];
        });
        updates.changed().await.unwrap();
        assert_eq!(
            link.view().sections[1].entries[1],
            SettingsEntry::Peers { count: 1 }
        );
    }

    /// 还没取到列表时的配对状态；用例里会改它。
    fn peers_view_model() -> PeersViewModel {
        PeersViewModel {
            state: PeersState::Loading,
            peers: Vec::new(),
            requests: Vec::new(),
            rejected: Vec::new(),
            code: None,
        }
    }

    fn friends_view_model() -> FriendsViewModel {
        FriendsViewModel {
            state: FriendsState::Loading,
            friends: Vec::new(),
            groups: Vec::new(),
            incoming: Vec::new(),
            outgoing: Vec::new(),
            blocked: Vec::new(),
            can_block_federation: false,
            public_access_off: false,
            my_link: None,
        }
    }

    /// 引导前后都不会变的一份聊天状态。
    fn chat_view_model() -> ChatViewModel {
        ChatViewModel {
            chat_layout_default: Default::default(),
            account: wonton_protocol::ChatAccount::SignedOut,
            sync: wonton_protocol::ChatSync::Idle,
            encryption: wonton_protocol::EncryptionStatus {
                verification: wonton_protocol::DeviceVerification::Unknown,
                recovery: RecoveryStatus::Unknown,
                recovery_key: None,
                backup_reminder: None,
                can_verify_with_other_device: false,
                can_reset: false,
                recovery_next_step: wonton_protocol::RecoveryNextStep::None,
                verification_flow: None,
                key_handoff: None,
            },
            installing_bots: Vec::new(),
            rooms: Vec::new(),
            problem: None,
            push: PushStatus::NoEndpoint,
            pending_confirmations: Vec::new(),
            settled_confirmations: Vec::new(),
            background_notifications: Vec::new(),
            bots: Vec::new(),
        }
    }
}
