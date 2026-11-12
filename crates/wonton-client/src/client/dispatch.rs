//! 用户操作分到各个作用域（ADR-0042）：这里是分派表，具体的活在各自的链路里。

use wonton_l10n::l10n;
use wonton_protocol::DeviceNetwork;

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
            _ => Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "servers-invalid-action"),
            }),
        }
    }

    /// 处理一个操作。会话操作成功时状态立即更新并推给所有订阅者；失败时状态不变，返回原因。
    ///
    /// 这个调用不阻塞，可以在主线程上直接调用。连接家里服务器这类网络操作在后台执行。
    pub fn dispatch(&self, action: Action) -> Result<(), CoreError> {
        let scope = self.scope();
        match action {
            action @ (Action::CreateSession { .. }
            | Action::SelectSession { .. }
            | Action::DeleteSession { .. }) => self.session_action(action),
            Action::ConnectHome { ticket } => scope.home.connect(&ticket),
            Action::PairHome { code, device_name } => scope.home.pair(code.expose(), &device_name),
            Action::InviteDevice => scope.home.invite(),
            action @ (Action::AddServer
            | Action::CancelAddServer
            | Action::SwitchServer { .. }
            | Action::RenameServer { .. }
            | Action::RemoveServer { .. }) => self.change_server(action),
            peers @ (Action::RefreshPeers
            | Action::ShowPeerCode
            | Action::HidePeerCode
            | Action::ResetPeerCode
            | Action::ScanPeerCode { .. }
            | Action::ApprovePeerRequest { .. }
            | Action::RejectPeerRequest { .. }
            | Action::UnrejectPeerRequest { .. }
            | Action::RemovePeer { .. }) => scope.peers.dispatch(peers),
            Action::AppForegrounded => {
                scope.foregrounded();
                Ok(())
            }
            Action::AppBackgrounded => {
                scope.backgrounded();
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
                self.register_others(&scope, endpoint.clone());
                self.replay().push = Some(PushInput::Endpoint(endpoint));
                Ok(())
            }
            Action::PushUnavailable { message } => {
                scope.push.unavailable(message.clone());
                self.replay().push = Some(PushInput::Unavailable(message));
                Ok(())
            }
            Action::SetPushMethod { method } => scope.push_method.set(method),
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
            action @ (Action::SelectTab { .. }
            | Action::CloseTabOverlay
            | Action::PinTab { .. }
            | Action::UnpinTab { .. }
            | Action::MoveTab { .. }
            | Action::SetTabLabels { .. }
            | Action::SetTabDisplay { .. }
            | Action::EnterFullscreen { .. }
            | Action::ExitFullscreen { .. }
            | Action::ReloadTab { .. }
            | Action::TabLoadFailed { .. }
            | Action::TabLoadSucceeded { .. }
            | Action::DismissUninstalledTabs) => scope.tabs.dispatch(action),
            other => self.dispatch_plugins_or_chat(&scope, other),
        }
    }
}
