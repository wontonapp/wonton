//! 联邦转发的监听地址解析（ADR-0050）：入站协议转给的 Tuwunel 联邦端口按连接现解析（入口容器重建后
/// 地址会变，见 [`crate::entry_containers`]），出站代理的监听地址启动时解析一次。
use std::net::SocketAddr;

use anyhow::Context;

/// 解析出站代理的监听地址。别名只在核心的内部网络上，解析出来的就是核心在那个网络上的地址；有 IPv4
/// 就用 IPv4（compose 的网络不开 IPv6，Tuwunel 解析别名拿到的也是它）。
pub(crate) async fn listen(listen: &str) -> anyhow::Result<SocketAddr> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host(listen)
        .await
        .with_context(|| format!("解析 WONTON_FEDERATION_LISTEN 的地址 {listen}"))?
        .collect();
    addrs
        .iter()
        .find(|addr| addr.is_ipv4())
        .or(addrs.first())
        .copied()
        .ok_or_else(|| anyhow::anyhow!("WONTON_FEDERATION_LISTEN 的 {listen} 解析不出地址"))
}
