//! 用户操作分到各个作用域（ADR-0042）：这里是分派表，具体的活在各自的链路里。

#[cfg(feature = "ui-fake")]
use std::sync::atomic::Ordering;

use wonton_l10n::l10n;
use wonton_protocol::{DeviceNetwork, PlacedTab, TabRef};

use super::switch::PushInput;
use super::*;
use crate::l10n::{CATALOG, language};

#[uniffi::export]
impl ClientCore {
    fn session_action(&self, action: Action) -> Result<(), CoreError> {
        match action {
            Action::CreateSession { title } => self.update_sessions(|store| store.create(&title)),
            Action::SelectSession { session_id } => {
                self.update_sessions(|store| store.select(session_id))
            }
            Action::DeleteSession { session_id } => {
                self.update_sessions(|store| store.delete(&session_id))
            }
            _ => Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "sessions-invalid-action"),
            }),
        }
    }

    /// 服务器列表的操作：增、改、切换、移除（ADR-0042）。
    fn change_server(&self, action: Action) -> Result<(), CoreError> {
        match action {
            Action::AddServer => self.change_servers(ServerList::add),
            Action::CancelAddServer => self.change_servers(ServerList::cancel_add),
            Action::SwitchServer { id } => self.change_servers(|servers| servers.switch(&id)),
            Action::RenameServer { id, name } => {
                self.change_servers(|servers| servers.rename(&id, &name))
            }
            Action::RemoveServer { id } => self.change_servers(|servers| servers.remove(&id)),
            action if is_matrix_login_action(&action) => self.matrix_login.dispatch(action),
            _ => Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "servers-invalid-action"),
            }),
        }
    }

    /// 处理一个操作。会话操作成功时状态立即更新并推给所有订阅者；失败时状态不变，返回原因。
    ///
    /// 这个调用不阻塞，可以在主线程上直接调用。连接家里服务器这类网络操作在后台执行。
    pub fn dispatch(&self, action: Action) -> Result<(), CoreError> {
        // 假数据入口（`new_ui_fake`）没有后端：动作不落到作用域，免得把塞进去的 ViewModel 又改回空。
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Ok(());
        }
        let scope = self.scope();
        match action {
            action if is_sharing_action(&action) => self.dispatch_sharing(&scope, action),
            action @ (Action::CreateSession { .. }
            | Action::SelectSession { .. }
            | Action::DeleteSession { .. }) => self.session_action(action),
            Action::ConnectHome { ticket } => scope.home.connect(&ticket),
            Action::PairHome { code, device_name } => self.pair_home(code.expose(), &device_name),
            Action::DismissPairingFailure => scope.home.dismiss_pairing_failure(),
            Action::CancelPairing => scope.home.cancel_pairing(),
            action
            @ (Action::InviteDevice | Action::CloseDeviceInvite | Action::SkipKeyHandoff) => {
                key_handoff_action(&scope, &action)
            }
            action @ (Action::AddServer
            | Action::CancelAddServer
            | Action::SwitchServer { .. }
            | Action::RenameServer { .. }
            | Action::RemoveServer { .. }) => self.change_server(action),
            action if is_matrix_login_action(&action) => self.change_server(action),
            peers @ (Action::RefreshPeers
            | Action::ShowPeerCode
            | Action::HidePeerCode
            | Action::ResetPeerCode
            | Action::ScanPeerCode { .. }
            | Action::ApprovePeerRequest { .. }
            | Action::RejectPeerRequest { .. }
            | Action::UnrejectPeerRequest { .. }
            | Action::RemovePeer { .. }) => scope.peers.dispatch(peers),
            contacts @ (Action::RefreshBotContacts { .. }
            | Action::RequestBotContact { .. }
            | Action::RequestPersonContact { .. }
            | Action::ApproveBotContact { .. }
            | Action::RejectBotContact { .. }
            | Action::RemoveBotContact { .. }) => scope.bot_contacts.dispatch(contacts),
            friends @ (Action::RefreshFriends
            | Action::AddFriend { .. }
            | Action::ApproveFriendRequest { .. }
            | Action::RejectFriendRequest { .. }
            | Action::CancelFriendRequest { .. }
            | Action::RemoveFriend { .. }
            | Action::BlockFriend { .. }
            | Action::UnblockFriend { .. }
            | Action::SaveFriendGroup { .. }
            | Action::RemoveFriendGroup { .. }
            | Action::SetFriendGroups { .. }) => scope.friends.dispatch(friends),
            action @ (Action::AppForegrounded
            | Action::AppBackgrounded
            | Action::NetworkChanged { .. }
            | Action::RetryConnection
            | Action::SetPushEndpoint { .. }
            | Action::PushUnavailable { .. }
            | Action::SetPushMethod { .. }) => self.lifecycle_action(&scope, action),
            action @ (Action::RefreshOnboarding
            | Action::RestartOnboarding
            | Action::ChooseOnboardingAgent { .. }
            | Action::ContinueOnboarding
            | Action::SkipOnboardingStep
            | Action::BackOnboardingStep
            | Action::ToggleOnboardingNeed { .. }
            | Action::EditOnboardingProfile { .. }
            | Action::TestModelProvider { .. }
            | Action::UseCurrentModel
            | Action::ChooseModelLogin { .. }
            | Action::RetryOnboardingItem { .. }
            | Action::DismissModelProviderProblem
            | Action::SetPrimaryAgent { .. }
            | Action::DismissReplacedAgentOffer
            | Action::DismissOnboardingTip { .. }) => scope.onboarding.dispatch(action),
            action @ (Action::UploadFile { .. }
            | Action::DownloadFile { .. }
            | Action::CancelTransfer { .. }
            | Action::DismissTransfer { .. }
            | Action::ResendFileLink { .. }
            | Action::DeleteFile { .. }
            | Action::SendFileToDevices { .. }
            | Action::GrantFileToRoom { .. }
            | Action::DismissFileProblem) => scope.files.dispatch(action),
            action @ (Action::RefreshCoreUpgrade { .. }
            | Action::StartCoreUpgrade { .. }
            | Action::CancelCoreUpgrade
            | Action::SkipCoreUpgrade { .. }) => scope.plugins.dispatch_core_upgrade(action),
            Action::SetAccentColor { color } => self.appearance.set_accent(color),
            Action::SetThemeMode { mode } => self.appearance.set_theme_mode(mode),
            Action::SetTextSize { size } => self.appearance.set_text_size(size),
            Action::SetFont { font } => self.appearance.set_font(font),
            Action::Timeline { action } if super::composer::is_composer_action(&action) => {
                self.composer_action(&scope, action)
            }
            action if is_page_action(&action) => scope.pages.dispatch(action),
            other => self.dispatch_plugins_or_chat(&scope, other),
        }
    }
}

/// 页面和面板自己的操作，整组交给 [`PagesLink`](crate::pages::PagesLink)。
/// 分享页的加载结果也走这里记状态；来源重探在 sharing 那边做。
fn is_page_action(action: &Action) -> bool {
    matches!(
        action,
        Action::OpenPage { .. }
            | Action::ClosePage
            | Action::AddMyApp { .. }
            | Action::RemoveMyApp { .. }
            | Action::MoveMyApp { .. }
            | Action::SetAutoAddApps { .. }
            | Action::AddPanelPage { .. }
            | Action::RemovePanelPage { .. }
            | Action::MovePanelPage { .. }
            | Action::SelectPanelPage { .. }
            | Action::SetBotPinConsent { .. }
            | Action::PinCurrentPage { .. }
            | Action::ReloadTab { .. }
            | Action::TabLoadFailed { .. }
            | Action::TabLoadSucceeded { .. }
            | Action::DismissUninstalledTabs
            | Action::SetRoomEntryBlocked { .. }
    )
}

/// 加设备时交接聊天密钥（ADR-0087）：“添加设备”页开着时，聊天那边自动接上一次新设备的确认请求。
fn key_handoff_action(scope: &Scope, action: &Action) -> Result<(), CoreError> {
    match action {
        Action::InviteDevice => {
            scope.home.invite()?;
            scope.chat.open_device_invite();
        }
        Action::CloseDeviceInvite => {
            scope.home.close_invite();
            scope.chat.close_device_invite();
        }
        _ => return scope.chat.skip_key_handoff(),
    }
    Ok(())
}

/// 分享相关的操作，整组交给 `dispatch_sharing`。分享页的加载结果也算：先重探来源，
/// 再转给页面状态。
fn is_sharing_action(action: &Action) -> bool {
    matches!(
        action,
        Action::RefreshSharedApps { .. }
            | Action::OpenSharedApp { .. }
            | Action::CreatePluginShare { .. }
            | Action::UpdatePluginShare { .. }
            | Action::RevokePluginShare { .. }
            | Action::TabLoadFailed {
                page: PlacedTab::Page {
                    tab: TabRef::SharedApp { .. },
                },
                ..
            }
            | Action::TabLoadSucceeded {
                page: PlacedTab::Page {
                    tab: TabRef::SharedApp { .. },
                },
            }
    )
}

/// 用现成 Matrix 账号登录的操作（ADR-0051），整组交给 [`MatrixLogins`](crate::matrix_login::MatrixLogins)。
fn is_matrix_login_action(action: &Action) -> bool {
    matches!(
        action,
        Action::StartMatrixLogin { .. }
            | Action::SubmitMatrixPassword { .. }
            | Action::StartMatrixOidc { .. }
            | Action::CompleteMatrixOidc { .. }
            | Action::CancelMatrixLogin { .. }
    )
}

impl ClientCore {
    /// 前台、网络和推送方式这些会话级的操作。
    fn lifecycle_action(&self, scope: &Scope, action: Action) -> Result<(), CoreError> {
        match action {
            Action::AppForegrounded => {
                // 先判锁再叫醒各部件：锁定页盖上之前不放任何一页先动。
                self.app_lock.foregrounded();
                scope.foregrounded();
                Ok(())
            }
            Action::AppBackgrounded => {
                scope.backgrounded();
                self.app_lock.backgrounded(scope.handle());
                Ok(())
            }
            Action::NetworkChanged { network } => {
                self.replay().network = Some(network);
                scope.home.network_changed(network);
                if network != DeviceNetwork::None {
                    // 连接可能没断、只是迁移了路径：同步和发送队列的退避不用等完。
                    scope.chat.foregrounded();
                }
                Ok(())
            }
            Action::RetryConnection => scope.home.retry(),
            Action::SetPushEndpoint { endpoint } => {
                scope.push.set_endpoint(endpoint.clone())?;
                self.register_others(scope, endpoint.clone());
                self.replay().push = Some(PushInput::Endpoint(endpoint));
                Ok(())
            }
            Action::PushUnavailable { message } => {
                scope.push.unavailable(message.clone());
                self.replay().push = Some(PushInput::Unavailable(message));
                Ok(())
            }
            Action::SetPushMethod { method } => scope.push_method.set(method),
            _ => Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "sessions-invalid-action"),
            }),
        }
    }
}
