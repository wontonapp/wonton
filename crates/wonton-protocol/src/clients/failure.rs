//! 第三方客户端接口失败的原因，壳据此提示主人下一步。

use serde::{Deserialize, Serialize};

/// 错误响应里写类别的响应头，值是 [`ClientsFailure::as_str`]，和其他管理接口同一个头（ADR-0017 的格式）。
pub const CLIENTS_ERROR_HEADER: &str = "X-Wonton-Error";

/// 失败的原因；每个变体的说明里是核心用的 HTTP 状态码。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ClientsFailure {
    /// 不是经隧道的已配对设备，或者不是主人的设备：入口的开关、域名和公网入口只有主人能改（401）。
    Unauthenticated,
    /// 插件没装、没在运行，或者核心查不出 `setup_done`，附命令的标准错误输出（503，ADR-0022）。
    PluginUnavailable,
    /// 应用的命令失败或超时，附标准错误输出（502）。超时也算失败，但可能已经生效，按待完成重试。
    CredentialsFailed,
    /// 输入不合法：域名、服务商名、凭据变量名、公开路径、有效期（400）。
    InvalidInput,
    /// 现在做不了：`credentials` 还没成功发过密码就打开入口，或者 `per_client: false` 的插件再建密码（409）。
    Conflict,
    /// 局域网地址选不出，或者 `*.lan` 的 A 记录不对，写明要加的那一条（422）。
    LanUnavailable,
    /// 中继的 TCP 端口池分完了（409）。
    PortQuotaExceeded,
    /// 这个家还没注册，要先填邀请码（409）。
    NotRegistered,
    /// 应用的首次注册没完成，入口打不开（409，ADR-0022）。
    SetupPending,
    /// 核心跑在宿主上，没有 `core` 网络，入口监听和隧道绑定做不到（422）。
    CoreNotInContainer,
    /// 局域网和公网入口都没开，建不了访客链接（409，ADR-0090 第 5 节）。
    EntryClosed,
    /// 连接器起不来或连不上，附详情（503）。
    IngressUnavailable,
    /// 公网访问总开关关着（409）。
    PublicAccessOff,
    /// 比这个 App 新的核心给的类别。
    #[serde(other)]
    Other,
}

impl ClientsFailure {
    /// 类别的写法。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unauthenticated => "unauthenticated",
            Self::PluginUnavailable => "plugin_unavailable",
            Self::CredentialsFailed => "credentials_failed",
            Self::InvalidInput => "invalid_input",
            Self::Conflict => "conflict",
            Self::LanUnavailable => "lan_unavailable",
            Self::PortQuotaExceeded => "port_quota_exceeded",
            Self::NotRegistered => "not_registered",
            Self::SetupPending => "setup_pending",
            Self::CoreNotInContainer => "core_not_in_container",
            Self::EntryClosed => "entry_closed",
            Self::IngressUnavailable => "ingress_unavailable",
            Self::PublicAccessOff => "public_access_off",
            Self::Other => "other",
        }
    }

    /// 用哪个 HTTP 状态码返回这个类别；[`ClientsFailure::Other`] 只在壳里出现，核心不拿它回响应。
    pub fn http_status(self) -> Option<u16> {
        match self {
            Self::Unauthenticated => Some(401),
            Self::PluginUnavailable | Self::IngressUnavailable => Some(503),
            Self::CredentialsFailed => Some(502),
            Self::InvalidInput => Some(400),
            Self::Conflict
            | Self::PortQuotaExceeded
            | Self::NotRegistered
            | Self::SetupPending
            | Self::EntryClosed
            | Self::PublicAccessOff => Some(409),
            Self::LanUnavailable | Self::CoreNotInContainer => Some(422),
            Self::Other => None,
        }
    }
}

/// 客户端接口的错误响应体：`{"error": {"kind": …, "message": …}}`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClientsErrorBody {
    pub error: ClientsProblem,
}

/// 错误体里的一条：类别加核心按 App 的语言写的说明（ADR-0043）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClientsProblem {
    pub kind: ClientsFailure,
    pub message: String,
}
