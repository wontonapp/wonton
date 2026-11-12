//! 插件在客户端页上的入口、开关和打不开的原因（ADR-0021“入口的开关”）。

use serde::{Deserialize, Serialize};

use super::passwords::PasswordSummary;

/// 清单声明的公开路径规则（ADR-0021“公网暴露面”、ADR-0090 第 4、6 节）；只对公网入口生效。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PublicRules {
    pub mode: PublicRuleMode,
    /// 清单里的原文，每条以 `/` 开头；放行的按前缀匹配，挡掉的按完整路径匹配。
    pub paths: Vec<String>,
}

/// 公网上放行哪几条路径，或者挡掉哪几条；两者清单里只写一个。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PublicRuleMode {
    /// 公网上只有这几条能到应用，其余只在局域网入口和 App 里能用（要求的应用带 `auth: app`）。
    Allow,
    /// 公网上除了这几条都能到应用（如博客挡掉后台和注册）。
    Deny,
    /// 比这个 App 新的核心给的写法。
    #[serde(other)]
    Other,
}

/// 一个入口的种类；每个声明了 `clients.http` 的插件有局域网和公网两个，每个 `clients.tcp` 一个。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntryKind {
    /// 只在局域网里可达，地址是 `<标签>.lan.<后缀>:<局域网端口>`。
    Lan,
    /// 公网可达，地址是 `<标签>.<后缀>`；还受总开关管。
    Public,
    /// `clients.tcp` 的一项，端口由中继或路由器分出。
    Tcp {
        /// 清单里声明的名字，`[a-z0-9-]`，最长 32。
        name: String,
    },
    /// 比这个 App 新的核心给的入口。
    #[serde(other)]
    Other,
}

impl EntryKind {
    /// 拼进 [`clients_entry_path`](crate::clients_entry_path) 的那一段；[`EntryKind::Other`] 拼不出来。
    pub fn as_path_segment(&self) -> Option<&str> {
        match self {
            Self::Lan => Some("lan"),
            Self::Public => Some("public"),
            Self::Tcp { name } => Some(name),
            Self::Other => None,
        }
    }
}

/// 一个入口，以及现在开着没有。线上是 `{"kind": {"kind": "tcp", "name": "ssh"}, "enabled": false}`：
/// 外面的 `kind` 是这一行的字段，里面的 `kind` 是 [`EntryKind`] 自己的标签。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct EntrySwitch {
    pub kind: EntryKind,
    pub enabled: bool,
}

/// 声明了 `clients` 或 `webui.guests` 的一个插件。全部默认关，只有主人在 App 里能开（ADR-0021“入口的开关”）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClientsPlugin {
    pub id: String,
    /// 插件的显示名。
    pub label: String,
    /// 局域网地址；局域网入口开着时才有。
    pub lan_url: Option<String>,
    /// 公网地址；公网入口开着、总开关也开着时才有。
    pub public_url: Option<String>,
    /// 先是局域网，再是公网，最后按名字排 `clients.tcp`。
    pub entries: Vec<EntrySwitch>,
    pub auth: ClientAuthKind,
    /// 给应用的登录用户名；`app` 模式下为空，`credentials` 的默认值是主人的 Matrix 用户名。
    pub username: Option<String>,
    /// 专用密码，按新建时间排；`app` 模式下为空。
    pub passwords: Vec<PasswordSummary>,
    /// 清单声明的公开路径；没声明时为空。
    pub public_rules: Option<PublicRules>,
    /// 现在开不了时为什么；能开时为空。
    pub blocked: Option<EntryBlock>,
}

/// 谁验证访问（ADR-0021“专用密码”），决定 App 里显示密码还是只说“用应用自己的账号”。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ClientAuthKind {
    /// 网关验证专用密码，通过后把用户名写进请求头给应用。
    Gateway,
    /// 核心在应用里发密码，应用自己验证。
    Credentials,
    /// 应用自己的账号，核心不管密码。
    App,
    /// 比这个 App 新的核心给的验证方式。
    #[serde(other)]
    Other,
}

/// 一个入口现在开不了的原因，壳按类别写文字；能开时是空的。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum EntryBlock {
    /// 应用的首次注册还没完成，入口打不开（ADR-0022）。
    SetupPending,
    /// `credentials` 模式还没成功发过一次密码；发成功前入口打不开。
    CredentialsNotIssued,
    /// 局域网地址选不出，或者 `*.lan` 的 A 记录不对。
    LanUnavailable,
    /// 公网访问总开关关着，公网入口暂时不生效（各插件的开关还留着）。
    PublicAccessOff,
    /// 还没有域名，证书签不出来。
    NoDomain,
    /// 公网入口还没配好（没选入口种类，或者连接器没起来）。
    IngressNotConfigured,
    /// 比这个 App 新的核心给的原因。
    #[serde(other)]
    Other,
}

/// [`clients_entry_path`](crate::clients_entry_path) 的请求体。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SetEntryRequest {
    pub enabled: bool,
}
