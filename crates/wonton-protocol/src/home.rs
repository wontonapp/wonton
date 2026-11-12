use serde::{Deserialize, Serialize};

use crate::SecretString;

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
    /// 要告诉主人的连接状态；连接正常时为空。壳只按它显示，不看 `status` 里的原文（ADR-0032）。
    pub notice: Option<ConnectionNotice>,
    /// 连接页的诊断和指标。
    pub details: ConnectionDetails,
    /// 本地代理；没在运行时为空。WebView 的 HTTP 代理要设成它，并带上凭据。
    pub proxy: Option<ProxySettings>,
    /// 插件网页的入口地址（M0 由连接信息给出）。
    pub web_url: Option<String>,
    /// 扫码配对的进展；没在配对时为空，配上后停在 [`PairingStatus::Paired`] 直到下一次配对或重置连接信息。
    pub pairing: Option<PairingStatus>,
    /// “添加设备”的进展和给新设备扫的码（[`Action::InviteDevice`](crate::Action::InviteDevice)）；没在添加设备时为空。
    pub invite: Option<DeviceInvite>,
    /// 已配对、收新消息的方式是 [`PushMethod::Keepalive`](crate::PushMethod::Keepalive)：Android 的壳按它启停“保持连接”的前台服务。
    pub keep_connection: bool,
    /// 断过又连上了：短暂为真，期间壳把短状态换成“已连接”；什么时候清由核心掌握，壳不计时。
    #[serde(default)]
    pub just_recovered: bool,
}

/// 给新设备扫的配对码。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeviceInvite {
    /// 正在经隧道向家里的服务器要码。
    Requesting,
    /// 新设备扫 `code`（`wonton://pair?…`）：壳把它画成二维码，也显示文字给不能扫码的设备粘贴。码只能用一次，
    /// 到 `expires_at_ms`（Unix 毫秒）过期。
    Ready {
        code: SecretString,
        expires_at_ms: u64,
    },
    Failed {
        reason: HomeFailure,
        message: String,
    },
}

/// 配对进行到哪一步：类别由核心给，壳据此写“还在连接”一行的话。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PairingStage {
    /// 正在建立到服务器的连接（连中继、握手、交出配对码），服务器还没有收到码。
    Connecting,
    /// 配对码已经交出，正在等服务器登记并回应。
    Confirming,
}

/// 扫码配对的进展。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PairingStatus {
    /// 正在连服务器、出示配对码。`elapsed_ms` 是这一次已经进行的时间，核心按秒推进；壳按它决定何时补
    /// “还在连接”一行和取消按钮，不在壳里自己计时。取消走 [`crate::Action::CancelPairing`]，
    /// 进行中的每一步都能取消：本机只是放弃这一次尝试，服务端按设备幂等登记，同一个码再配对仍能配上。
    InProgress {
        elapsed_ms: u64,
        stage: PairingStage,
    },
    /// 配上了：服务器登记了这台设备，连接信息存下了，连接正在建立。壳看到它就离开配对页；
    /// 取消不会到这一步。下一次开始配对、或者连接信息被重置时清掉。
    Paired,
    Failed {
        reason: PairingFailure,
        /// 原始说明，壳只在“详情”里显示。被服务器拒绝的几类（`Rejected`、`Expired`、`Closed`）是服务器按它主人的语言
        /// 写的，和类别说的是一件事，壳不显示，按类别自己写标题和下一步。
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
    /// 服务器不再接受这种码，例如已经有配对的设备时的首次配对码。要在已配对的设备上添加，或在服务器上另发一个码。
    Closed,
    /// 连不上服务器：手机和服务器之间不通，或者服务器没在运行。
    TransportUnavailable,
    /// 服务器出错了，稍后再试；原因在 `message` 里。
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
    /// 连接失败或断开。隔一会儿自动再试，试的时候回到 [`Self::Connecting`]；`Unauthorized` 不再试，要主人重新登记
    /// 这台设备。
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

/// 手机现在怎么上网，来自系统的网络回调。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum DeviceNetwork {
    /// 系统说没有可用网络。
    None,
    Wifi,
    Cellular,
    /// 有线、VPN 等其他网络。
    Other,
}

/// 要告诉主人的连接状态。前三种是短状态，替换页面标题；其余的在标题下面多一行，带操作（ADR-0032）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConnectionNotice {
    /// 系统说没有网络。
    WaitingForNetwork,
    /// 刚断开，正在重连。
    Connecting,
    /// 连上了，正在补上断开期间的消息。
    Updating,
    /// 断开很久了，手机能上网，家里的服务器没有回应。`retrying` 表示主人点的“重试”正在进行。
    ServerOffline { retrying: bool },
    /// 断开很久了，手机也连不上中继，分不清是哪一边的问题。
    Unreachable { retrying: bool },
    /// 服务器不再接受这台设备，要重新配对。
    DeviceRejected,
    /// 两端协议版本不匹配，App 或服务器要更新。
    VersionMismatch,
}

/// 连接质量（ADR-0032）：核心按最近窗口的丢包率和往返时间判定，壳只在连接着且为 [`Self::Poor`] 时提示。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ConnectionQuality {
    /// 正常。
    Good,
    /// 连着但很差：丢包多或往返慢。
    Poor,
    /// 样本不够，或者还没连上。
    #[default]
    Unknown,
    /// 比这个 App 新的取值，按 `Unknown` 显示。
    #[serde(other)]
    Other,
}

/// 连接页的一行诊断。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ConnectionCheck {
    Ok,
    Failing,
    /// 还不知道，或者这种配置下无从判断。
    #[default]
    Unknown,
}

/// 连接页的诊断和指标。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConnectionDetails {
    /// 手机的网络；壳还没报告过时为空。
    pub network: Option<DeviceNetwork>,
    /// 手机连不连得上中继。
    pub relay: ConnectionCheck,
    /// 家里的服务器有没有回应。
    pub server: ConnectionCheck,
    /// 最后一次确认连上的时间（Unix 毫秒）；这次启动还没连上过时为空。
    pub last_connected_at_ms: Option<u64>,
    /// 最近一次建连的耗时，毫秒。
    pub handshake_ms: Option<u32>,
    /// 这次启动以来断开的次数。
    pub disconnects: u32,
    /// 距上次连上，这一轮的建连已经失败过几次；连上就清零。判断“这次断开是本轮新失败”用。
    #[serde(default)]
    pub failed_dials: u32,
    /// 当前连接发出的包数。
    pub sent_packets: u64,
    /// 当前连接判定丢失的包数。
    pub lost_packets: u64,
    /// 最近判定窗口的连接质量。
    #[serde(default)]
    pub quality: ConnectionQuality,
    /// 判定窗口内的丢包率，千分比（`50` 是 5%）；样本不够时为空。
    #[serde(default)]
    pub loss_per_mille: Option<u16>,
    /// 判定窗口内的往返时间，毫秒；样本不够时为空。
    #[serde(default)]
    pub rtt_ms: Option<u32>,
    /// 最近一次失败的原文，只在“详情”里显示；连上之后还留着，方便查刚才的问题。
    pub last_failure: Option<String>,
    /// 连上之后一直没取到服务器现在的地址：存着的地址可能已经过时，下次建连可能要先绕中继。取到就清掉。
    pub addresses_stale: bool,
    /// 当前隧道连接的 UDP 流向；只统计完整应用包，不能与上面的 QUIC 包计数相加。
    #[serde(default)]
    pub udp_flows: Vec<crate::UdpFlowStats>,
    /// 连接级被发送缓冲推出的数据报帧数估计；无法测得时为空，分片可能让一包占两帧。
    #[serde(default)]
    pub udp_buffer_evicted_datagrams: Option<u64>,
}

/// 连接当前走的路径。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct HomePath {
    /// 经中继转发；否则是局域网或打洞成功的直连。
    pub relay: bool,
    /// 直连的对端是局域网地址。
    pub lan: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 上一版的 JSON 里没有质量字段和窗口数字，新核心要读得懂，缺的按不知道处理。
    #[test]
    fn details_without_quality_fields_read_as_unknown() {
        let details: ConnectionDetails = serde_json::from_str(
            r#"{"network":"wifi","relay":"ok","server":"ok","last_connected_at_ms":1793778000000,
                "handshake_ms":120,"disconnects":2,"sent_packets":900,"lost_packets":3,
                "last_failure":null,"addresses_stale":false}"#,
        )
        .unwrap();
        assert_eq!(details.quality, ConnectionQuality::Unknown);
        assert_eq!(details.loss_per_mille, None);
        assert_eq!(details.rtt_ms, None);
    }

    #[test]
    fn unknown_quality_falls_back_to_other() {
        assert_eq!(
            serde_json::from_str::<ConnectionQuality>(r#""hologram""#).unwrap(),
            ConnectionQuality::Other
        );
    }

    #[test]
    fn quality_round_trips_through_json() {
        let details = ConnectionDetails {
            quality: ConnectionQuality::Poor,
            loss_per_mille: Some(73),
            rtt_ms: Some(1_240),
            ..ConnectionDetails::default()
        };
        let json = serde_json::to_string(&details).unwrap();
        let read: ConnectionDetails = serde_json::from_str(&json).unwrap();
        assert_eq!(read.quality, ConnectionQuality::Poor);
        assert_eq!(read.loss_per_mille, Some(73));
        assert_eq!(read.rtt_ms, Some(1_240));
    }
}
