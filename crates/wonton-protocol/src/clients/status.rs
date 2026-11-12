//! 入口状态和自检（ADR-0021“接口”“公网暴露面”、ADR-0090 第 7 节）：App 的设置页和好友页按这里的类别写文字。

use serde::{Deserialize, Serialize};

use super::domain::DomainSettings;
use super::entries::PublicRules;
use super::ingress::{IngressSettings, PublicAccess, PublicSwitchState};

/// `GET` [`clients_status_path`](crate::clients_status_path) 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClientsStatus {
    /// 核心在容器里跑才做得到入口的监听和隧道绑定；在宿主上时打开入口回 `core_not_in_container`。
    pub core_in_container: bool,
    pub domain: DomainSettings,
    pub ingress: IngressSettings,
    pub public_access: PublicAccess,
    /// 总开关关着时三道闸门各自做没做完。
    pub gates: PublicSwitchState,
    /// 最近一次自检：从家里按公网地址绕出去再连回自己；只说明这条路通，不代表外面所有网络都通。
    pub self_probe: ProbeResult,
    /// 别的服务器上的好友现在能不能从公网联系到这台主机；好友页和跨服务器联系人按它显示。
    pub public_reach: PublicReach,
    /// 现在公开的东西：开着公网入口的应用、它的路径规则和还有效的访客链接。
    pub public_now: Vec<PublicItem>,
    /// 最近的封禁，按开始时间倒序。
    pub bans: Vec<Ban>,
    /// 还没做完的事（待换值、待吊销、待生效的 CAA）。吊销没完成时相应密码的
    /// [`PasswordSummary::pending`](crate::PasswordSummary) 也标着。
    pub pending: Vec<PendingChange>,
}

/// 别的服务器上的好友现在能不能从公网联系到这台主机。总开关关着时如实说，不显示成“对方不在线”。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PublicReach {
    /// 对方能联系到，联邦和回调都通。
    Reachable,
    /// 总开关关着：别的服务器上的人联系不到这台，重新打开后对方重试才到。
    PublicAccessOff,
    /// 连接器断了：外面现在也联系不到。
    ConnectorDown,
    /// 还没有域名。
    NoDomain,
    /// 公网入口还没配好。
    IngressNotConfigured,
    /// 还没自检成功过，或者查不出来。
    Unknown,
    /// 比这个 App 新的核心给的状态。
    #[serde(other)]
    Other,
}

/// 最近一次自检的结果；失败的原因分开显示，不合并成“连不上”。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ProbeResult {
    Ok,
    /// 证书快要续期了（有效期已经短于续期窗口）。
    CertExpiring,
    CertExpired,
    /// 公网地址解析到的不是家里。
    DnsWrong,
    /// 中继或者连接器不通。
    ConnectorDown,
    /// 插件没在运行。
    PluginNotRunning,
    /// 应用的首次注册状态查不出来（ADR-0022 的 `setup_done`）。
    SetupUnknown,
    /// 还没有结果：刚打开，或者公网入口关着。
    NotRun,
    /// 比这个 App 新的核心给的结果。
    #[serde(other)]
    Other,
}

/// 现在公开的一个应用。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PublicItem {
    pub plugin_id: String,
    pub label: String,
    /// 公网上的路径规则；没有时为空。
    pub rules: Option<PublicRules>,
    /// 还有效的访客链接的名字。
    pub guest_links: Vec<String>,
}

/// 一次封禁；按来源（IPv4 地址或 IPv6 的 /64），到点自动解除，主人也能提前解。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Ban {
    pub source: String,
    pub since_ms: u64,
    pub until_ms: u64,
    /// 封之前这 5 分钟里这个来源失败了几次。
    pub failures: u32,
}

/// 一件还没做完、核心会重试的事；App 如实显示成待完成。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PendingChange {
    pub kind: PendingChangeKind,
    /// 涉及的应用；没有时为空（如 CAA 变更）。
    pub plugin_id: Option<String>,
    /// 核心按 App 的语言写的详情，如失败命令的标准错误输出。
    pub detail: Option<String>,
}

/// 待完成的是哪一件。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PendingChangeKind {
    /// 待吊销的密码：对账不碰它，只重试 `revoke`。
    CredentialRevoke,
    /// 待换值的密码：新值已经记下，插件启动时先重跑。
    CredentialRotate,
    /// 待生效的 CAA 变更。
    CaaChange,
    /// 比这个 App 新的核心给的事情。
    #[serde(other)]
    Other,
}
