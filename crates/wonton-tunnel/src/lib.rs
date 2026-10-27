//! iroh 1.x 隧道，服务端与客户端共用。
//!
//! 服务端只接受已配对设备的 endpoint ID；客户端在本机提供 HTTP 代理，把 Matrix 请求和小程序流量
//! 转发到家里服务器的核心网关。设计见 `docs/adr/0004-iroh-tunnel.md`。
