//! 隧道错误，以及把 iroh / QUIC 错误归类成调用方能据以行动的几种原因。

use iroh::endpoint::{
    ConnectError, ConnectWithOptsError, ConnectingError, ConnectionError, ReadError,
    ReadExactError, WriteError,
};

use crate::wire::CLOSE_UNAUTHORIZED;

/// 隧道操作失败的原因。
///
/// 调用方据此决定怎么做：`Unauthorized` 需要重新配对；`TransportUnavailable` 稍后会自动重试；
/// `UpstreamUnavailable` 说明家里的服务器连上了，但上面的服务没就绪。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TunnelError {
    /// 服务端拒绝了本设备：endpoint ID 不在已配对白名单里，或已被吊销。
    #[error("the server rejected this device: endpoint ID not paired or revoked")]
    Unauthorized,
    /// 连不上服务端，或连接中断、超时。下一次使用时会自动重连。
    #[error("tunnel transport unavailable: {reason}")]
    TransportUnavailable {
        /// 底层错误的描述。
        reason: String,
    },
    /// 隧道是通的，但服务端连不上它要转发到的上游 HTTP 服务。
    #[error("the server cannot reach its upstream HTTP service")]
    UpstreamUnavailable,
    /// 对端违反隧道协议，通常是两端版本不匹配。
    #[error("tunnel protocol error: {reason}")]
    Protocol {
        /// 违反协议的具体情况。
        reason: String,
    },
    /// 本地的隧道客户端或 endpoint 已经关闭。
    #[error("the tunnel client is closed")]
    Closed,
}

impl TunnelError {
    /// 稳定的机器可读错误码，本地代理把它放进 `x-wonton-tunnel-error` 响应头。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::TransportUnavailable { .. } => "transport-unavailable",
            Self::UpstreamUnavailable => "upstream-unavailable",
            Self::Protocol { .. } => "protocol",
            Self::Closed => "closed",
        }
    }

    /// 用户可读的错误类别，本地代理把它放进 `x-wonton-error` 响应头：壳按类别显示人话，不读错误
    /// 原文（ADR-0032）。比 [`Self::code`] 粗：传输不可用和客户端已关闭对用户是同一回事。取值是
    /// 两端写文案的依据，改了要连着改 iOS 和 Android 的本地化资源。
    pub fn category(&self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::TransportUnavailable { .. } | Self::Closed => "tunnel-unavailable",
            Self::UpstreamUnavailable => "upstream-unavailable",
            Self::Protocol { .. } => "protocol",
        }
    }

    pub(crate) fn transport(reason: impl std::fmt::Display) -> Self {
        Self::TransportUnavailable {
            reason: reason.to_string(),
        }
    }

    pub(crate) fn protocol(reason: impl Into<String>) -> Self {
        Self::Protocol {
            reason: reason.into(),
        }
    }
}

pub(crate) fn connection_log_code(err: &ConnectionError) -> &'static str {
    match err {
        ConnectionError::ApplicationClosed(close) if close.error_code == CLOSE_UNAUTHORIZED => {
            "unauthorized"
        }
        ConnectionError::ApplicationClosed(_) => "peer-closed",
        ConnectionError::LocallyClosed => "locally-closed",
        _ => "transport-unavailable",
    }
}

/// 连接被关闭的原因。服务端用 [`CLOSE_UNAUTHORIZED`] 关闭即为未授权；其余都是传输问题。
///
/// 本机断开的连接也算传输问题：客户端关闭后，新的请求由客户端自己回 [`TunnelError::Closed`]；回到前台时
/// 换掉的失效连接（[`crate::TunnelClient::revalidate`]），下一次请求会走新连接。
pub(crate) fn from_connection_error(err: &ConnectionError) -> TunnelError {
    match err {
        ConnectionError::ApplicationClosed(close) if close.error_code == CLOSE_UNAUTHORIZED => {
            TunnelError::Unauthorized
        }
        ConnectionError::LocallyClosed => TunnelError::transport("connection closed locally"),
        other => TunnelError::transport(other),
    }
}

pub(crate) fn from_connect_error(err: &ConnectError) -> TunnelError {
    match err {
        ConnectError::Connection { source, .. } => from_connection_error(source),
        ConnectError::Connecting {
            source: ConnectingError::ConnectionError { source, .. },
            ..
        } => from_connection_error(source),
        ConnectError::Connect {
            source: ConnectWithOptsError::EndpointClosed { .. },
            ..
        } => TunnelError::Closed,
        other => TunnelError::transport(format!("{other:#}")),
    }
}

pub(crate) fn from_read_error(err: &ReadError) -> TunnelError {
    match err {
        ReadError::ConnectionLost(source) => from_connection_error(source),
        other => TunnelError::transport(other),
    }
}

pub(crate) fn from_read_exact_error(err: &ReadExactError) -> TunnelError {
    match err {
        ReadExactError::ReadError(source) => from_read_error(source),
        ReadExactError::FinishedEarly(_) => {
            TunnelError::protocol("the server ended the stream without sending a status")
        }
    }
}

pub(crate) fn from_write_error(err: &WriteError) -> TunnelError {
    match err {
        WriteError::ConnectionLost(source) => from_connection_error(source),
        other => TunnelError::transport(other),
    }
}
