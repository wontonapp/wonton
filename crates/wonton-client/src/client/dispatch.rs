//! 用户操作分到各个作用域（ADR-0042）：这里是唯一的领域解包点。

use std::sync::atomic::{AtomicUsize, Ordering};

use wonton_l10n::l10n;
use wonton_protocol::{
    Action, AgentAction, BotAction, ChatAction, ClientsAction, DefaultBotVisibility, DeviceNetwork,
    HomeAction, PageAction, PluginAction, PushAction, SessionAction, SettingsAction,
    TimelineAction,
};

use super::switch::PushInput;
use super::*;
use crate::agent_chains::actions::Effect;
use crate::chat::ask::{AskRequest, DefaultBot};
use crate::l10n::{CATALOG, language};

/// 假数据下已经试过几次发送，`WONTON_UI_FAIL_SEND` 按它数前几次该拒。
static UI_FAKE_SENDS: AtomicUsize = AtomicUsize::new(0);

#[uniffi::export]
impl ClientCore {
    /// 假数据入口下照走真逻辑的时间线操作：拿掉附件只动本机的附件区，发送照样要过核心的检查。
    ///
    /// 只有走输入框的 `SendComposer` 查附件区：真核心的 `send_text` / `send_image` 都不看它
    /// （`chat/send.rs`），命令和旁聊贴图有空附件失败也照发。
    fn dispatch_ui_fake_composer(&self, action: &TimelineAction) -> Result<(), CoreError> {
        // `WONTON_UI_FAIL_SEND=<n>`：接下来 n 次发送都照附件传不上去的样子拒掉；数字缺省或不是数字时算 1，
        // 够测“第一张被拒、后面的照发”。没有这个变量就都照发。
        let is_send = matches!(
            action,
            TimelineAction::SendText { .. }
                | TimelineAction::SendImage { .. }
                | TimelineAction::SendComposer { .. }
        );
        if is_send {
            let failed = UI_FAKE_SENDS.fetch_add(1, Ordering::SeqCst);
            let limit = std::env::var("WONTON_UI_FAIL_SEND")
                .map_or(0, |value| value.trim().parse().unwrap_or(1));
            if failed < limit {
                return Err(CoreError::InvalidInput {
                    reason: l10n!(CATALOG, language(), "composer-has-failed"),
                });
            }
        }
        match action {
            TimelineAction::RemoveAttachment { .. } => {
                self.composer_action(&self.scope(), action.clone())
            }
            // 假数据没有真上传，附件区的失败由 `fail_attachment_for_test` 写；被拒就中止，不往假时间线补。
            // 过了拒绝才有附言：补进假时间线、清附件区，返回 Ok 让外层继续开假链。
            TimelineAction::SendComposer { target, body, .. } => {
                #[cfg(not(feature = "ui-fake"))]
                let _ = body;
                self.scope().composer.reject_if_failed(&target.room_id)?;
                self.scope().composer.clear_for_test(&target.room_id);
                #[cfg(feature = "ui-fake")]
                if !body.is_empty() {
                    self.ui_fake_timelines
                        .timeline_action(TimelineAction::SendText {
                            target: target.clone(),
                            body: body.clone(),
                            reply_to: None,
                            mentions: Vec::new(),
                        });
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// 假数据下的服务器开关：设置里“添加服务器”换成配对页，取消退出。
    #[cfg_attr(not(feature = "ui-fake"), allow(dead_code))]
    fn dispatch_ui_fake_home(&self, action: HomeAction) {
        match action {
            HomeAction::AddServer => self.servers.seed_adding_ui_fake(true),
            HomeAction::CancelAddServer => self.servers.seed_adding_ui_fake(false),
            _ => {}
        }
    }

    /// 处理一个操作。领域子枚举只在这里解包，运行时仍只有一个入口。
    pub fn dispatch(&self, action: Action) -> Result<(), CoreError> {
        // 发送（包括附件区整单）先停“正在输入”：假数据里发送被拒也算试过发送，所以要排在拒绝之前。
        if let Action::Timeline {
            action: TimelineAction::SendComposer { target, .. },
        } = &action
        {
            self.scope().chat.stop_typing(target);
        }
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return match action {
                Action::Pages { action } => self.dispatch_ui_fake(action),
                Action::Chat { action } => {
                    self.ui_fake_timelines.chat_action(&action);
                    self.dispatch_ui_fake_chat(action);
                    Ok(())
                }
                Action::Plugins { action } => self.dispatch_ui_fake_plugins(action),
                Action::Family { action } => {
                    self.dispatch_ui_fake_family(action);
                    Ok(())
                }
                // 假时间线（跳转、发字）和停止钮的假运行段都要看到同一个操作。发送的检查放在前面：
                // 先过发送的拒绝和附件区的检查：被拒的发送不进假时间线、不开假链，也不回显；收下了才记假时间线（发附言的顺带补一条）。
                Action::Timeline { action } => {
                    self.dispatch_ui_fake_composer(&action)?;
                    self.ui_fake_timelines.timeline_action(action.clone());
                    self.dispatch_ui_fake_timeline(action);
                    Ok(())
                }
                Action::Home { action } => {
                    self.dispatch_ui_fake_home(action);
                    Ok(())
                }
                Action::Agent { action } => {
                    self.dispatch_ui_fake_agent(action);
                    Ok(())
                }
                // 外观和语言全在本机，假数据下也要照实落盘，否则设置页点了没反应。
                Action::Settings { action } => self.settings_action(&self.scope(), action),
                _ => Ok(()),
            };
        }

        let scope = self.scope();
        match action {
            Action::Session { action } => self.session_action(action),
            Action::Home { action } => self.home_action(&scope, action),
            Action::Chat { action } => self.chat_action(&scope, action),
            Action::Bot { action } => self.bot_action(&scope, action),
            Action::Push { action } => self.push_action(&scope, action),
            Action::Plugins { action } => self.plugin_action(&scope, action),
            Action::Settings { action } => self.settings_action(&scope, action),
            Action::Onboarding { action } => scope.onboarding.dispatch(action),
            Action::Files { action } => scope.files.dispatch(action),
            Action::Pages { action } => self.page_action(&scope, action),
            Action::Peers { action } => scope.peers.dispatch(action),
            Action::BotContacts { action } => scope.bot_contacts.dispatch(action),
            Action::Family { action } => scope.family.dispatch(action),
            Action::Friends { action } => scope.friends.dispatch(action),
            Action::Sharing { action } => self.dispatch_sharing(&scope, action),
            Action::Timeline { action } => self.timeline_action(&scope, action),
            Action::Agent { action } => self.agent_action(&scope, action),
            Action::Clients { action } => match action {
                // 公网访问的模块还没接；接上之前如实报还没就绪，不静默当成功。
                ClientsAction::RefreshDomain
                | ClientsAction::CancelDomain
                | ClientsAction::SelectDomainProvider { .. }
                | ClientsAction::SaveDomain { .. }
                | ClientsAction::RetryCertificate
                | ClientsAction::DismissDomainProblem => scope.clients_domain.dispatch(action),
                _ => Err(CoreError::NotReady {
                    reason: "clients module is not wired yet".into(),
                }),
            },
        }
    }
}

impl ClientCore {
    fn session_action(&self, action: SessionAction) -> Result<(), CoreError> {
        match action {
            SessionAction::CreateSession { title } => {
                self.update_sessions(|store| store.create(&title))
            }
            SessionAction::SelectSession { session_id } => {
                self.update_sessions(|store| store.select(session_id))
            }
            SessionAction::DeleteSession { session_id } => {
                self.update_sessions(|store| store.delete(&session_id))
            }
        }
    }

    fn home_action(&self, scope: &Scope, action: HomeAction) -> Result<(), CoreError> {
        match action {
            HomeAction::ConnectHome { ticket } => scope.home.connect(&ticket),
            HomeAction::PairHome { code, device_name } => {
                self.pair_home(code.expose(), &device_name)
            }
            HomeAction::DismissPairingFailure => scope.home.dismiss_pairing_failure(),
            HomeAction::CancelPairing => scope.home.cancel_pairing(),
            HomeAction::InviteDevice => {
                scope.home.invite()?;
                scope.chat.open_device_invite();
                Ok(())
            }
            HomeAction::CloseDeviceInvite => {
                scope.home.close_invite();
                scope.chat.close_device_invite();
                Ok(())
            }
            action @ (HomeAction::AddServer
            | HomeAction::CancelAddServer
            | HomeAction::SwitchServer { .. }
            | HomeAction::RenameServer { .. }
            | HomeAction::RemoveServer { .. }) => self.change_server(action),
            action @ (HomeAction::StartMatrixLogin { .. }
            | HomeAction::SubmitMatrixPassword { .. }
            | HomeAction::StartMatrixOidc { .. }
            | HomeAction::CompleteMatrixOidc { .. }
            | HomeAction::CancelMatrixLogin { .. }) => self.matrix_login.dispatch(action),
            HomeAction::RelaySettings { action } => scope.home.relay_settings(action),
            action @ (HomeAction::AppForegrounded
            | HomeAction::AppBackgrounded
            | HomeAction::NetworkChanged { .. }
            | HomeAction::RetryConnection) => self.lifecycle_action(scope, action),
        }
    }

    fn change_server(&self, action: HomeAction) -> Result<(), CoreError> {
        match action {
            HomeAction::AddServer => self.change_servers(ServerList::add),
            HomeAction::CancelAddServer => self.change_servers(ServerList::cancel_add),
            HomeAction::SwitchServer { id } => self.change_servers(|servers| servers.switch(&id)),
            HomeAction::RenameServer { id, name } => {
                self.change_servers(|servers| servers.rename(&id, &name))
            }
            HomeAction::RemoveServer { id } => self.change_servers(|servers| servers.remove(&id)),
            _ => Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "servers-invalid-action"),
            }),
        }
    }

    fn chat_action(&self, scope: &Scope, action: ChatAction) -> Result<(), CoreError> {
        match action {
            ChatAction::AskAgent {
                text,
                room_id,
                request_id,
            } => {
                let onboarding = scope.onboarding.view();
                let default = match (
                    onboarding.default_bot_user_id.as_deref(),
                    onboarding.default_bot_visibility,
                ) {
                    (Some(bot), Some(DefaultBotVisibility::Visible)) => bot
                        .parse()
                        .map(DefaultBot::Visible)
                        .unwrap_or(DefaultBot::Unknown),
                    (_, Some(DefaultBotVisibility::Absent)) => DefaultBot::Absent,
                    _ => DefaultBot::Unknown,
                };
                scope.chat.ask(
                    AskRequest {
                        text,
                        room_id,
                        request_id,
                    },
                    default,
                )
            }
            ChatAction::DismissChatProblem => scope.chat.dispatch(ChatAction::DismissChatProblem),
            ChatAction::SkipKeyHandoff => scope.chat.skip_key_handoff(),
            action @ (ChatAction::JoinRoom { .. }
            | ChatAction::LeaveRoom { .. }
            | ChatAction::StartDirectChat { .. }
            | ChatAction::AnswerAgentChoice { .. }
            | ChatAction::ResetRecovery
            | ChatAction::ConfirmRecoveryKeySaved
            | ChatAction::RecoverEncryption { .. }
            | ChatAction::StartDeviceVerification
            | ChatAction::AcceptDeviceVerification { .. }
            | ChatAction::ConfirmVerification { .. }
            | ChatAction::DenyVerification { .. }
            | ChatAction::CancelDeviceVerification { .. }
            | ChatAction::CheckReady
            | ChatAction::AnswerConfirmation { .. }
            | ChatAction::StopConfirmationWindow { .. }
            | ChatAction::RetryConfirmations
            | ChatAction::SetChatLayout { .. }) => scope.chat.dispatch(action),
        }
    }

    fn bot_action(&self, scope: &Scope, action: BotAction) -> Result<(), CoreError> {
        match action {
            BotAction::SetBotName { plugin_id, name } => scope.chat.set_bot_name(plugin_id, name),
            BotAction::SetBotAvatar {
                plugin_id,
                state,
                upload,
            } => scope.chat.set_bot_avatar(plugin_id, state, upload),
            BotAction::ClearBotAvatar { plugin_id, state } => {
                scope.chat.clear_bot_avatar(plugin_id, state)
            }
            BotAction::DismissBotProfileProblem { plugin_id } => {
                scope.chat.dismiss_bot_profile_problem(&plugin_id);
                Ok(())
            }
        }
    }

    fn push_action(&self, scope: &Scope, action: PushAction) -> Result<(), CoreError> {
        match action {
            PushAction::SetPushEndpoint { endpoint } => {
                scope.push.set_endpoint(endpoint.clone())?;
                self.register_others(scope, endpoint.clone());
                self.replay().push = Some(PushInput::Endpoint(endpoint));
                Ok(())
            }
            PushAction::PushUnavailable { message } => {
                scope.push.unavailable(message.clone());
                self.replay().push = Some(PushInput::Unavailable(message));
                Ok(())
            }
            PushAction::SetPushMethod { method } => scope.push_method.set(method),
        }
    }

    fn plugin_action(&self, scope: &Scope, action: PluginAction) -> Result<(), CoreError> {
        match action {
            PluginAction::RefreshPlugins => {
                scope.plugins.refresh_installed();
                Ok(())
            }
            PluginAction::StartPlugin { plugin_id } => scope.plugins.start(plugin_id),
            PluginAction::SetPortMapping { plugin_id, enabled } => {
                scope.plugins.set_port_mapping(plugin_id, enabled)
            }
            PluginAction::RefreshStore { force } => {
                scope.plugins.refresh_store(force);
                Ok(())
            }
            PluginAction::InstallPlugin {
                preview,
                secrets,
                settings,
            } => scope.plugins.install(preview, secrets, settings),
            PluginAction::UpgradePlugin {
                preview,
                secrets,
                settings,
            } => scope.plugins.upgrade(preview, secrets, settings),
            PluginAction::RetryPluginInstall { id } => scope.plugins.retry_install(id),
            PluginAction::UninstallPlugin { id, delete_data } => {
                scope.plugins.uninstall(id, delete_data)
            }
            PluginAction::SetPluginAudience { id, audience } => {
                scope.plugins.set_plugin_audience(id, audience)
            }
            PluginAction::DismissPluginTask { id } => scope.plugins.dismiss_task(&id),
            PluginAction::InstallNow { source } => scope.plugins.install_now(source),
            PluginAction::UpgradeNow { plugin_id } => scope.plugins.upgrade_now(plugin_id),
            PluginAction::UpgradeAll => scope.plugins.upgrade_all(),
            PluginAction::ReviewStoreUpdate { plugin_id } => {
                scope.plugins.review_store_update(&plugin_id)
            }
            PluginAction::DismissStoreUpdates => scope.plugins.dismiss_store_updates(),
            PluginAction::DismissStoreConfirmation => {
                scope.plugins.dismiss_confirmation();
                Ok(())
            }
            PluginAction::StartTakeover { id } => {
                scope.plugins.start_takeover(id);
                Ok(())
            }
            PluginAction::EndTakeover { id } => {
                scope.plugins.end_takeover(&id);
                Ok(())
            }
            PluginAction::RefreshEgressProxy => {
                scope.plugins.refresh_egress();
                Ok(())
            }
            PluginAction::SaveEgressProxy {
                address,
                direct_cidrs,
                use_plugin,
            } => scope
                .plugins
                .save_egress(&address, &direct_cidrs, use_plugin),
            PluginAction::ConfirmEgressProxy => scope.plugins.confirm_egress(),
            PluginAction::DismissEgressConfirmation => {
                scope.plugins.dismiss_egress_confirmation();
                Ok(())
            }
            PluginAction::RefreshRegistryMirror => {
                scope.plugins.refresh_registry_mirror();
                Ok(())
            }
            PluginAction::SaveRegistryMirror { mirror } => {
                scope.plugins.save_registry_mirror(mirror)
            }
            PluginAction::TestRegistryMirrors => scope.plugins.test_registry_mirrors(),
            action @ (PluginAction::RefreshCoreUpgrade { .. }
            | PluginAction::StartCoreUpgrade { .. }
            | PluginAction::CancelCoreUpgrade
            | PluginAction::SkipCoreUpgrade { .. }) => scope.plugins.dispatch_core_upgrade(action),
            PluginAction::SetPluginMediaPermission { plugin_id, allow } => {
                scope.plugins.set_media_permission(plugin_id, allow)
            }
            action @ (PluginAction::ListOwnerSecrets { .. }
            | PluginAction::ShowOwnerSecret { .. }
            | PluginAction::HideOwnerSecret { .. }
            | PluginAction::DismissOwnerSecrets { .. }) => {
                scope.plugins.dispatch_owner_secrets(action)
            }
        }
    }

    fn settings_action(&self, scope: &Scope, action: SettingsAction) -> Result<(), CoreError> {
        match action {
            SettingsAction::SetLanguages { languages } => {
                #[cfg(feature = "test-hooks")]
                self.test_before_language_refresh();
                // The dispatch scope may already have been replaced. Take the switch lock
                // before obtaining the scope, and keep replay in the same ordering.
                let _switching = switch::lock(&self.switching);
                crate::l10n::set_languages(&languages);
                self.scope().set_languages(&languages);
                self.replay().languages = Some(languages);
                Ok(())
            }
            SettingsAction::SetThemeMode { mode } => self.appearance.set_theme_mode(mode),
            SettingsAction::SetTextSize { size } => self.appearance.set_text_size(size),
            SettingsAction::SetFont { font } => self.appearance.set_font(font),
            SettingsAction::SetLockScreenContent { category, hidden } => {
                scope.lock_screen.set(category, hidden)
            }
        }
    }

    fn page_action(&self, scope: &Scope, action: PageAction) -> Result<(), CoreError> {
        match action {
            PageAction::TabLoadFailed { page, status } => {
                if is_shared_page(&page) {
                    self.shared_page_failed(scope, page, status)
                } else {
                    scope
                        .pages
                        .dispatch(PageAction::TabLoadFailed { page, status })
                }
            }
            PageAction::TabLoadSucceeded { page } => {
                if is_shared_page(&page) {
                    self.shared_page_loaded(scope, page)
                } else {
                    scope.pages.dispatch(PageAction::TabLoadSucceeded { page })
                }
            }
            action @ (PageAction::OpenPage { .. }
            | PageAction::ClosePage
            | PageAction::AddMyApp { .. }
            | PageAction::RemoveMyApp { .. }
            | PageAction::MoveMyApp { .. }
            | PageAction::SetAutoAddApps { .. }
            | PageAction::RetryMyAppSync
            | PageAction::AddPanelPage { .. }
            | PageAction::RemovePanelPage { .. }
            | PageAction::MovePanelPage { .. }
            | PageAction::SelectPanelPage { .. }
            | PageAction::SetBotPinConsent { .. }
            | PageAction::PinCurrentPage { .. }
            | PageAction::ReloadTab { .. }
            | PageAction::DismissUninstalledTabs) => scope.pages.dispatch(action),
        }
    }

    fn timeline_action(&self, scope: &Scope, action: TimelineAction) -> Result<(), CoreError> {
        if super::composer::is_composer_action(&action) {
            self.composer_action(scope, action)
        } else {
            scope.chat.timeline_action(action)
        }
    }

    fn agent_action(&self, scope: &Scope, action: AgentAction) -> Result<(), CoreError> {
        if let AgentAction::StopTurns { target } = action {
            self.stop_turns(scope, target);
            return Ok(());
        }
        let (core_url, proxy) = super::agent_chains::connection(scope)?;
        let view = self.views().agent_chains.clone();
        view.send_if_modified(|model| model.action_failure.take().is_some());
        scope.spawn(async move {
            match crate::agent_chains::actions::perform(&core_url, &proxy, &action).await {
                Ok(Effect::Stopped(row)) => {
                    view.send_modify(|model| crate::agent_chains::apply_row(model, row))
                }
                Ok(Effect::Done) => {}
                Err(error) => view.send_modify(|model| {
                    model.action_failure =
                        Some(wonton_protocol::AgentActionFailure { action, error });
                }),
            }
        });
        Ok(())
    }

    fn lifecycle_action(&self, scope: &Scope, action: HomeAction) -> Result<(), CoreError> {
        match action {
            HomeAction::AppForegrounded => {
                tracing::info!("the shell reports the app came to the foreground");
                self.app_lock.foregrounded();
                scope.foregrounded();
                Ok(())
            }
            HomeAction::AppBackgrounded => {
                tracing::info!("the shell reports the app went to the background");
                scope.backgrounded();
                self.app_lock.backgrounded(scope.handle());
                Ok(())
            }
            HomeAction::NetworkChanged { network } => {
                self.replay().network = Some(network);
                scope.home.network_changed(network);
                if network != DeviceNetwork::None {
                    scope.chat.foregrounded();
                }
                Ok(())
            }
            HomeAction::RetryConnection => scope.home.retry(),
            _ => Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "sessions-invalid-action"),
            }),
        }
    }
}

fn is_shared_page(page: &wonton_protocol::PlacedTab) -> bool {
    matches!(
        page,
        wonton_protocol::PlacedTab::Page {
            tab: wonton_protocol::TabRef::SharedApp { .. },
        } | wonton_protocol::PlacedTab::Content {
            tab: wonton_protocol::TabRef::SharedApp { .. },
        }
    )
}
