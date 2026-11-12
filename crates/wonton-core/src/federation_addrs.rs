//! 联邦转发的两个地址（ADR-0050）：入站转给的 Tuwunel 联邦端口，和出站代理的监听地址。

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, bail};
use wonton_core::config::Config;

/// 配了入口时联邦先经入口容器；入口不在（容器被删、刚升级、建失败）就退回直连 Tuwunel，
/// 联邦照样通，不用等入口起来。入口回来的下一次启动再经它（ADR-0021 的入口只做 TLS 和来源）。
const DIRECT: &str = "tuwunel.wonton.internal:8448";
const TRIES: u32 = 30;

/// 解析 Tuwunel 的联邦地址（主机名用时解析，这里一次）：容器刚起来时名字可能还没注册，重试一阵再失败。
pub(crate) async fn upstream(cfg: &Config) -> anyhow::Result<SocketAddr> {
    let addr = cfg.matrix_federation_addr.as_str();
    for attempt in 1..=TRIES {
        match tokio::net::lookup_host(addr).await {
            Ok(mut resolved) => match resolved.next() {
                Some(upstream) => return Ok(upstream),
                None => tracing::warn!(attempt, "联邦地址 {addr} 没有解析出任何地址"),
            },
            Err(e) if attempt < TRIES => {
                tracing::debug!(attempt, error = %e, "联邦地址 {addr} 还解析不了");
            }
            Err(e) => return Err(anyhow::anyhow!("解析联邦地址 {addr} 失败：{e}")),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    if addr != DIRECT
        && let Some(direct) = resolve_once(DIRECT).await
    {
        tracing::warn!(%addr, %DIRECT, "入口地址解析不了，联邦先直连 Tuwunel");
        return Ok(direct);
    }
    bail!("联邦地址 {addr} 一直解析不了：{TRIES} 次重试都失败")
}

async fn resolve_once(addr: &str) -> Option<SocketAddr> {
    tokio::net::lookup_host(addr)
        .await
        .ok()
        .and_then(|mut addrs| addrs.next())
}

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
