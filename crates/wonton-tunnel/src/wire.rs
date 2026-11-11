//! 隧道线协议 v0。
//!
//! 一条 QUIC 连接（ALPN 为 [`ALPN`]）承载双向流和 UDP 数据报；`Forward` 流对应本地代理的
//! TCP 连接，`Port` 流对应插件 TCP 端口，`Udp` 流用于建立和维持数据报流向。
//!
//! - 客户端开流后先写 1 字节请求类型（[`Request`]），`Forward` 之后紧跟原样转发的 TCP 字节。QUIC 的流要等
//!   发起方写入数据后对端才看得到，这个字节同时起 dumbpipe 握手字节的作用。
//! - 服务端先回 1 字节状态（[`Status`]）。`Ok` 之后按请求类型继续；其他状态之后直接结束流。
//! - 连接级错误用 QUIC 的 application close code 表达：服务端拒绝未配对或已吊销的设备时用 [`CLOSE_UNAUTHORIZED`]。
//!
//! `Port`/`Udp` 扩展沿用同一 ALPN；旧服务端不认识请求字节时回 `UnsupportedRequest`。
//! 未接入端口转发的服务端也回该状态，不接受请求或分配流向。未知状态在旧客户端是协议错误，
//! 不能当作 `Ok`。UDP 成功应答为状态 `Ok` 后紧跟 [`datagram::FlowId`] 的 QUIC 变长编码；
//! 双向流的生命周期约束该流向，载荷走独立的 QUIC DATAGRAM，不进入这条流。

pub mod datagram;
mod port;

pub use port::{MAX_PLUGIN_ID_LEN, MAX_PORT_REQUEST_LEN, PortRequest};

/// 线协议输入或调用方缓冲区不符合编码约束。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    #[error("truncated wire header")]
    Truncated,
    #[error("request is not Port or Udp")]
    InvalidRequest,
    #[error("invalid plugin identifier")]
    InvalidPlugin,
    #[error("port must be at least 1024")]
    InvalidPort,
    #[error("output buffer is too small")]
    BufferTooSmall,
    #[error("flow identifier exceeds 16383 or uses an unsupported QUIC integer width")]
    InvalidFlowId,
    #[error("invalid fragment flags or empty fragment")]
    InvalidFragment,
    #[error("packet exceeds the two-fragment budget")]
    PacketTooLarge,
    #[error("datagram limit cannot hold the header")]
    DatagramTooSmall,
    #[error("fragments do not belong to the same packet or repeat an index")]
    MismatchedFragments,
}

use iroh::endpoint::VarInt;

/// 隧道协议的 ALPN；扩展请求类型和状态不改变 ALPN，未知请求以状态拒绝。
pub const ALPN: &[u8] = b"wonton/tunnel/0";

/// 服务端拒绝设备（未配对或已吊销）时关闭连接用的错误码。
pub(crate) const CLOSE_UNAUTHORIZED: VarInt = VarInt::from_u32(403);
pub(crate) const REASON_UNAUTHORIZED: &[u8] = b"unauthorized";

/// 客户端主动关闭连接用的错误码。
pub(crate) const CLOSE_NORMAL: VarInt = VarInt::from_u32(0);

/// 转发中途出错时复位流用的错误码，让对端知道数据被截断，而不是正常结束。
pub(crate) const STREAM_ABORTED: VarInt = VarInt::from_u32(1);

/// 每条流的第一个字节：客户端请求的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Request {
    /// 把流的剩余字节原样转发到服务端的上游 HTTP 服务。
    Forward = 1,
    /// 只做一次往返，确认连接可用且本设备已获授权。
    Ping = 2,
    /// 转发到插件声明的 TCP 端口，头格式见 [`PortRequest`]。
    Port = 3,
    /// 为插件声明的 UDP 端口建立流向，头格式见 [`PortRequest`]。
    Udp = 4,
}

impl Request {
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Forward),
            2 => Some(Self::Ping),
            3 => Some(Self::Port),
            4 => Some(Self::Udp),
            _ => None,
        }
    }
}

/// 服务端在每条流上回的第一个字节。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Status {
    /// 请求已接受；`Forward` 之后是上游的原样字节。
    Ok = 0,
    /// 服务端连不上上游 HTTP 服务。
    UpstreamUnavailable = 1,
    /// 服务端不认识这个请求类型（客户端比服务端新）。
    UnsupportedRequest = 2,
    /// 请求指定的插件没有安装。
    PluginNotInstalled = 3,
    /// 插件未声明该端口与传输协议的组合。
    PortNotDeclared = 4,
    /// 插件已安装但不在运行。
    PluginNotRunning = 5,
}

impl Status {
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Ok),
            1 => Some(Self::UpstreamUnavailable),
            2 => Some(Self::UnsupportedRequest),
            3 => Some(Self::PluginNotInstalled),
            4 => Some(Self::PortNotDeclared),
            5 => Some(Self::PluginNotRunning),
            _ => None,
        }
    }
}
