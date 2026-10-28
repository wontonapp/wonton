//! 隧道线协议 v0。
//!
//! 一条 QUIC 连接（ALPN 为 [`ALPN`]）承载任意多条双向流，每条流对应本地代理收到的一条 TCP 连接，
//! 做法参考 dumbpipe：流里原样转发 TCP 字节。
//!
//! - 客户端开流后先写 1 字节请求类型（[`Request`]），`Forward` 之后紧跟原样转发的 TCP 字节。QUIC 的流要等
//!   发起方写入数据后对端才看得到，这个字节同时起 dumbpipe 握手字节的作用。
//! - 服务端先回 1 字节状态（[`Status`]）。`Ok` 之后是上游返回的原样字节；其他状态之后直接结束流。
//! - 连接级错误用 QUIC 的 application close code 表达：服务端拒绝未配对或已吊销的设备时用
//!   [`CLOSE_UNAUTHORIZED`]。

use iroh::endpoint::VarInt;

/// 隧道协议的 ALPN。协议不兼容地变化时升级末尾的版本号。
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
pub(crate) enum Request {
    /// 把流的剩余字节原样转发到服务端的上游 HTTP 服务。
    Forward = 1,
    /// 只做一次往返，确认连接可用且本设备已获授权。
    Ping = 2,
}

impl Request {
    pub(crate) fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Forward),
            2 => Some(Self::Ping),
            _ => None,
        }
    }
}

/// 服务端在每条流上回的第一个字节。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Status {
    /// 请求已接受；`Forward` 之后是上游的原样字节。
    Ok = 0,
    /// 服务端连不上上游 HTTP 服务。
    UpstreamUnavailable = 1,
    /// 服务端不认识这个请求类型（客户端比服务端新）。
    UnsupportedRequest = 2,
}

impl Status {
    pub(crate) fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Ok),
            1 => Some(Self::UpstreamUnavailable),
            2 => Some(Self::UnsupportedRequest),
            _ => None,
        }
    }
}
