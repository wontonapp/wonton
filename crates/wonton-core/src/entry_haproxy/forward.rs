//! 局域网转发的 HAProxy 配置：TCP 模式，不解 TLS，带 PROXY v2 交给入口的局域网前端（ADR-0021“局域网直连”）。
//! 写进去的只有类型化的地址、网段和端口，没有能注入配置的字符串。

use std::net::{Ipv4Addr, SocketAddrV4};

use ipnet::Ipv4Net;

/// 一份局域网转发配置要的值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forward {
    /// 转发在 `lanpub` 上的地址和容器里的端口：只听这一个地址，`lanfwd` 上不听。
    pub listen: SocketAddrV4,
    /// 放行的来源：选中的局域网地址所在的网段。
    pub allow: Ipv4Net,
    /// `lanpub` 的网桥网关。经 docker-proxy 进来的连接（包括公网 IPv6）来源都是它（ADR-0019 实测），它又可能
    /// 正好落在放行的网段里，所以先单独拒掉。
    pub deny: Ipv4Addr,
    /// 入口在 `lanfwd` 上的局域网前端。
    pub entry: SocketAddrV4,
}

pub fn render_forward(forward: &Forward) -> String {
    let Forward {
        listen,
        allow,
        deny,
        entry,
    } = forward;
    format!(
        "global\n  maxconn 1024\n\ndefaults\n  mode tcp\n  timeout connect 5s\n  timeout client 60s\n  timeout server 60s\n\n\
         frontend lan\n  bind {listen}\n  tcp-request connection reject if {{ src {deny} }}\n  \
         tcp-request connection reject unless {{ src {allow} }}\n  default_backend entry\n\n\
         backend entry\n  server entry {entry} send-proxy-v2\n"
    )
}
