//! 插件清单共用的局域网与隧道端口声明。

use serde::{Deserialize, Serialize};

/// 插件服务开放的端口；局域网与本机隧道映射均使用同号端口。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(deny_unknown_fields)]
pub struct PortSpec {
    pub service: String,
    pub port: u16,
    #[serde(default)]
    pub protocol: PortProtocol,
}

impl PortSpec {
    /// 端口与协议的显示键；交给 Docker 前须拒绝未知协议。
    pub fn key(&self) -> String {
        format!("{}/{}", self.port, self.protocol.as_str())
    }
}

/// 端口的传输协议，未知值保留为不可执行的 `Other`。
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "lowercase")]
pub enum PortProtocol {
    #[default]
    Tcp,
    Udp,
    #[serde(other)]
    Other,
}

impl PortProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
            Self::Other => "other",
        }
    }
}

/// 插件允许已配对设备经隧道访问的端口声明。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(deny_unknown_fields)]
pub struct TunnelSpec {
    /// 每个 `(port, protocol)` 在插件内只能出现一次：隧道请求和本机同号监听不携带服务名。
    pub ports: Vec<PortSpec>,
}
