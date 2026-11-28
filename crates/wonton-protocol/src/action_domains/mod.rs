mod family;
mod session;
mod settings;

mod file;
mod onboarding;
mod pages;
use serde::{Deserialize, Serialize};

use crate::{
    DeviceNetwork, MatrixOidcCallback, MatrixOidcLogin, MatrixPasswordLogin, RelaySettingsAction,
    SecretString,
};
mod bot_contacts;
mod bot_profile;
mod chat;
mod clients;
mod peers;
mod plugin;
mod push;
pub use bot_contacts::BotContactAction;
pub use bot_profile::BotAction;
pub use chat::ChatAction;
pub use clients::ClientsAction;
mod sharing;
pub use family::FamilyAction;
mod friends;
pub use file::FileAction;
pub use friends::FriendsAction;
pub use onboarding::OnboardingAction;
pub use pages::PageAction;
pub use peers::PeerAction;
pub use plugin::PluginAction;
pub use push::PushAction;
pub use session::SessionAction;
pub use settings::SettingsAction;
pub use sharing::SharingAction;

/// 家庭服务器、服务器列表、生命周期和外部 Matrix 登录的操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HomeAction {
    /// 不经配对，直接用给定的连接信息连接家里的服务器，并存在本机，之后每次启动自动连接。App 走
    /// [`HomeAction::PairHome`]，连接信息由核心按配对码生成；这个操作给测试用。连接信息不合法时返回
    /// `InvalidInput`；连接的进展和结果体现在 [`HomeViewModel`](crate::HomeViewModel) 里。
    ConnectHome {
        ticket: String,
    },
    /// 扫码配对，成功后保存连接信息；配对码或设备名不合法时返回 `InvalidInput`。
    /// 进展和结果见 [`HomeViewModel::pairing`](crate::HomeViewModel::pairing)。
    PairHome {
        code: SecretString,
        device_name: String,
    },
    /// 收起本次配对失败，返回扫码或粘贴；不取消正在进行的配对、不移除已保存的服务器。
    DismissPairingFailure,
    /// 取消正在进行的配对，回到扫码或粘贴；没有正在进行的配对时返回 `InvalidInput`。本机放弃这一次尝试，
    /// 服务器可能已经按交出的码登记了这台设备：同一个码再配对，服务端幂等地回成功，不会卡在“码已用过”。
    CancelPairing,
    /// “添加设备”：经隧道要一个一次性配对码给新设备扫（ADR-0009），页面开着时自动接上新设备的确认请求（ADR-0087）。
    /// 没配对时返回 `NotReady`；进展和码在 `HomeViewModel::invite`。再发一次换新码，旧码到期前仍能用。
    InviteDevice,
    CloseDeviceInvite,
    /// 添加一台服务器：在一个新位置上走配对，当前那台的连接先断开、数据留着。配对成功后新的那台成为当前这台；
    /// 配对前可以 [`HomeAction::CancelAddServer`] 回去（ADR-0042）。
    AddServer,
    /// 放弃添加，回到原来那台服务器。没在添加时返回 `InvalidInput`。
    CancelAddServer,
    /// 换到另一台配过的服务器：断开当前这台（进行中的传输暂停，换回来接着传），连上那台。`id` 是
    /// [`ServerRow::id`](crate::ServerRow::id)。
    SwitchServer {
        id: String,
    },
    /// 给配过的服务器改名，只存在这台手机上。名字去掉首尾空白后不能为空。
    RenameServer {
        id: String,
        name: String,
    },
    /// 从这台手机上移除一台服务器：删掉它在手机上的全部数据，不吊销这台设备（吊销在服务器那边）。移除的是当前这台时
    /// 换到剩下的第一台；一台都不剩时回到配对页。
    RemoveServer {
        id: String,
    },
    /// 开始添加外部 Matrix 账号，None 使用 matrix.org；方式和进展由服务器列表的 matrix_login 给出。
    StartMatrixLogin {
        homeserver: Option<String>,
    },
    SubmitMatrixPassword {
        login: MatrixPasswordLogin,
    },
    StartMatrixOidc {
        login: MatrixOidcLogin,
    },
    CompleteMatrixOidc {
        callback: MatrixOidcCallback,
    },
    CancelMatrixLogin {
        login_id: String,
    },
    /// App 回到前台。核心检查隧道和本地代理是否还能用，必要时重建；系统挂起 App 期间可能回收了套接字。
    /// Matrix 同步停了的话也在这时重启。
    AppForegrounded,
    /// App 进了后台。核心停止探测和自动重连，直到 [`HomeAction::AppForegrounded`]；连接不主动断，后台音乐和正在传的文件
    /// 还在用它。
    AppBackgrounded,
    /// 系统报告网络变了。壳每次收到系统回调都发，不做判断：核心让 iroh 重新检查路径，并立刻重试（ADR-0032）。
    NetworkChanged {
        network: DeviceNetwork,
    },
    /// 主人点了“重试”：清零退避，立刻试一次。
    RetryConnection,
    /// 中继表设置页（ADR-0048）上的操作：取表、加、改、删、挪档、设置 n0 兜底、保存、丢弃。
    /// 进展和结果在 [`HomeViewModel::relays`](crate::HomeViewModel::relays)。
    RelaySettings {
        action: RelaySettingsAction,
    },
}
