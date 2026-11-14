//! 用户操作分到各个作用域（ADR-0042）：这里是唯一的领域解包点。

#[cfg(feature = "ui-fake")]
use std::sync::atomic::Ordering;

use wonton_l10n::l10n;
use wonton_protocol::{
    Action, AgentAction, BotAction, ChatAction, DeviceNetwork, HomeAction, PageAction,
    PluginAction, PushAction, SessionAction, SettingsAction, TimelineAction,
};

use super::switch::PushInput;
use super::*;
use crate::agent_chains::actions::Effect;
use crate::l10n::{CATALOG, language};

#[uniffi::export]
impl ClientCore {
    /// 处理一个操作。领域子枚举只在这里解包，运行时仍只有一个入口。
    pub fn dispatch(&self, action: Action) -> Result<(), CoreError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return match action {
                Action::Pages { action } => self.dispatch_ui_fake(action),
                Action::Chat { action } => {
                    self.dispatch_ui_fake_chat(action);
                    Ok(())
                }
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
            Action::Family { action } => scope.plugins.dispatch_roles(action),
            Action::Friends { action } => scope.friends.dispatch(action),
            Action::Sharing { action } => self.dispatch_sharing(&scope, action),
            Action::Timeline { action } => self.timeline_action(&scope, action),
            Action::Agent { action } => self.agent_action(&scope, action),
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
            ChatAction::AskAgent { text } => {
                let bot = scope.onboarding.view().primary_bot_user_id.ok_or_else(|| {
                    CoreError::NotReady {
                        reason: l10n!(CATALOG, language(), "chat-no-default-agent"),
                    }
                })?;
                scope.chat.ask(&bot, &text)
            }
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
            | ChatAction::DismissChatProblem
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
                crate::l10n::set_languages(&languages);
                scope.set_languages(&languages);
                self.replay().languages = Some(languages);
                Ok(())
            }
            SettingsAction::SetAccentColor { color } => self.appearance.set_accent(color),
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
            | PageAction::SetRoomEntryBlocked { .. }
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
                self.app_lock.foregrounded();
                scope.foregrounded();
                Ok(())
            }
            HomeAction::AppBackgrounded => {
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
        }
    )
}
