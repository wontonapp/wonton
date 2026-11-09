//! 插件端口的地址由嵌入方解析，隧道不依赖 Docker 或插件安装记录。

use std::{fmt::Debug, future::Future, net::SocketAddr};

use wonton_protocol::PortProtocol;

use crate::wire::Status;

/// 已授权设备请求插件端口时，嵌入方核对声明和运行状态后给出插件网络里的地址。
///
/// 只能解析清单明确声明的 `(plugin, port, protocol)`，不能把插件名当作任意 DNS 目标。
/// 不认识的协议必须拒绝；解析失败不能伪装成插件不存在或没有运行。
pub trait PortResolver: Debug + Send + Sync + 'static {
    fn resolve(
        &self,
        plugin: &str,
        port: u16,
        protocol: PortProtocol,
    ) -> impl Future<Output = Result<SocketAddr, PortResolveError>> + Send;
}

/// 端口解析的拒绝类别，与连接到已解析地址之后的 I/O 错误分开。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PortResolveError {
    #[error("plugin port forwarding is not supported")]
    Unsupported,
    #[error("plugin is not installed")]
    PluginNotInstalled,
    #[error("plugin did not declare this port and protocol")]
    PortNotDeclared,
    #[error("plugin service is not running")]
    PluginNotRunning,
    #[error("plugin service address is unavailable")]
    UpstreamUnavailable,
}

impl PortResolveError {
    pub const fn status(self) -> Status {
        match self {
            Self::Unsupported => Status::UnsupportedRequest,
            Self::PluginNotInstalled => Status::PluginNotInstalled,
            Self::PortNotDeclared => Status::PortNotDeclared,
            Self::PluginNotRunning => Status::PluginNotRunning,
            Self::UpstreamUnavailable => Status::UpstreamUnavailable,
        }
    }
}
