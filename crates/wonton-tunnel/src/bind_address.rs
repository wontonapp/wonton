//! 隧道 UDP 套接字绑定的地址（`WONTON_TUNNEL_BIND`）的解析规则。

use std::net::SocketAddr;

/// 解析绑定的 IPv4 地址。正式安装写的是核心自己在某个网络上的别名：Docker 的内嵌 DNS 只把
/// 这个名字解析到那个网络里的地址，所以绑定的就是那个网络上的地址。取第一个 IPv4；没有就报错，
/// 失败即停。Docker 行为测试的探针（examples/bind-probe.rs）也走这条规则。
pub async fn resolve_bind_v4(host: &str, port: u16) -> anyhow::Result<SocketAddr> {
    tokio::net::lookup_host((host, port))
        .await?
        .find(SocketAddr::is_ipv4)
        .ok_or_else(|| anyhow::anyhow!("绑定名 {host} 没有 IPv4 地址"))
}
