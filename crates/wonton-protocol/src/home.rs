use serde::{Deserialize, Serialize};

/// 和家里服务器的连接：隧道状态，以及 WebView 和 HTTP 客户端要用的本地代理。壳只渲染，不做推导。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct HomeViewModel {
    /// 本机设备的 endpoint ID（64 位十六进制），主人凭它登记这台设备。
    pub device_id: String,
    /// 家里服务器的 endpoint ID；还没有连接信息时为空。
    pub server_id: Option<String>,
    pub status: HomeStatus,
    /// 当前连接走的路径；没有可用连接时为空。
    pub path: Option<HomePath>,
    /// 本地代理；没在运行时为空。WebView 的 HTTP 代理要设成它，并带上凭据。
    pub proxy: Option<ProxySettings>,
    /// 插件网页的入口地址（M0 由连接信息给出）。
    pub web_url: Option<String>,
    /// 扫码配对的进展；没在配对时为空，配对成功后也回到空。
    pub pairing: Option<PairingStatus>,
}

/// 扫码配对的进展。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PairingStatus {
    /// 正在连服务器、出示配对码。
    InProgress,
    Failed {
        reason: PairingFailure,
        message: String,
    },
}

/// 配对失败的原因，壳据此提示主人下一步。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PairingFailure {
    /// 服务器不认这个码：没发过，或者已经用过。要拿一个新码。
    Rejected,
    /// 码过期了，要拿一个新码。
    Expired,
    /// 服务器不再接受这种码，例如已经有配对的设备时的首次配对码。要用别的办法加这台设备，`message` 里有说明。
    Closed,
    /// 连不上服务器：手机和服务器之间不通，或者服务器没在运行。
    TransportUnavailable,
    /// 服务器出错了，稍后再试；`message` 里有原因。
    Upstream,
    /// 两端协议版本不匹配：App 或服务器要更新。
    Protocol,
}

/// 连接状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HomeStatus {
    /// 还没有连接信息，等 [`Action::ConnectHome`](crate::Action::ConnectHome)。
    NotConfigured,
    /// 正在建立连接，或者连上了但服务端还没确认本设备。
    Connecting,
    /// 隧道可用，服务端已确认本设备。
    Connected,
    /// 连接失败或断开。下一次使用时自动重连；`Unauthorized` 要主人重新登记这台设备。
    Disconnected {
        reason: HomeFailure,
        message: String,
    },
}

/// 连接失败的原因，壳据此决定怎么提示。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum HomeFailure {
    /// 服务端拒绝本设备：没有登记或已被吊销。
    Unauthorized,
    /// 连不上服务端，或者连接中断、超时。
    TransportUnavailable,
    /// 隧道是通的，但服务端上的服务没有就绪。
    UpstreamUnavailable,
    /// 两端协议版本不匹配。
    Protocol,
}

/// 连接当前走的路径。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct HomePath {
    /// 经中继转发；否则是局域网或打洞成功的直连。
    pub relay: bool,
    /// 往返时间估计，毫秒。
    pub rtt_ms: u32,
}

/// 本地代理：HTTP 代理地址和本次启动的会话凭据（`Proxy-Authorization: Basic`）。凭据只给本 App 的
/// WebView 和 HTTP 客户端，同机其他 App 拿不到，也就借用不了本设备的配对身份。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ProxySettings {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// 家里服务器的基础域名。WebView 只让它和它的子域名走代理（iOS 的 `matchDomains`、Android 的反向
    /// 绕过规则），别的地址直连；代理本身也只转发这个域名下的地址。
    pub domain: String,
}

impl std::fmt::Debug for ProxySettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxySettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("domain", &self.domain)
            .finish()
    }
}
