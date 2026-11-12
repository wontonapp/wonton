//! 隧道服务端示例：只接受白名单里的设备，把隧道流量转发到本机的 HTTP 服务。
//!
//! ```text
//! cargo run -p wonton-tunnel --example tunnel-server -- \
//!     --upstream 127.0.0.1:8080 --allow <客户端 endpoint ID>
//! ```
//!
//! 使用 n0 的公共中继和地址发现，客户端只凭服务端的 endpoint ID 就能连上。Ctrl-C 退出。

#[path = "common/mod.rs"]
mod common;

use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    path::PathBuf,
};

use anyhow::Result;
use clap::Parser;
use iroh::{Endpoint, EndpointId, endpoint::presets};
use wonton_tunnel::{Allowlist, TunnelServer};

/// Wonton 隧道服务端示例。
#[derive(Debug, Parser)]
struct Args {
    /// 隧道流量要转发到的本机 HTTP 服务。
    #[arg(long)]
    upstream: SocketAddr,
    /// 已配对设备的 endpoint ID，可以重复。
    #[arg(long = "allow")]
    allow: Vec<EndpointId>,
    /// 服务端私钥文件，不存在时自动生成；私钥决定 endpoint ID。默认 `$XDG_STATE_HOME/wonton/tunnel-server.key`
    /// （没设时 `~/.local/state/wonton/tunnel-server.key`），不放当前目录。
    #[arg(long)]
    key: Option<PathBuf>,
    /// 固定 UDP 端口（IPv4 和 IPv6 同端口），便于防火墙放行。默认随机。
    #[arg(long, default_value_t = 0)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    common::init_tracing();
    let args = Args::parse();
    let key =
        common::load_or_create_key(&common::key_path(args.key.clone(), "tunnel-server.key")?)?;
    let mut builder = Endpoint::builder(presets::N0).secret_key(key);
    if args.port != 0 {
        builder = builder
            .bind_addr((Ipv4Addr::UNSPECIFIED, args.port))?
            .bind_addr((Ipv6Addr::UNSPECIFIED, args.port))?;
    }
    let endpoint = builder.bind().await?;
    println!("服务端 endpoint ID：{}", endpoint.id());
    if args.allow.is_empty() {
        println!("白名单为空，所有设备都会被拒绝；用 --allow <endpoint ID> 加入设备。");
    }
    let server = TunnelServer::spawn(endpoint.clone(), Allowlist::new(args.allow), args.upstream);
    // 不把“上线”当作启动条件：连上中继之前，局域网内的客户端也能直连。上线后打印一次完整地址。
    tokio::spawn(endpoint.closed().run_until(report_online(endpoint.clone())));

    tokio::signal::ctrl_c().await?;
    println!("正在关闭……");
    server.shutdown().await;
    Ok(())
}

async fn report_online(endpoint: Endpoint) {
    endpoint.online().await;
    let addr = endpoint.addr();
    let relays: Vec<String> = addr.relay_urls().map(ToString::to_string).collect();
    let ips: Vec<String> = addr.ip_addrs().map(ToString::to_string).collect();
    println!("已连上中继：{}", relays.join(" "));
    println!("直连地址：{}", ips.join(" "));
    if let Some(relay) = relays.first() {
        println!("客户端可加参数：--relay {relay}");
    }
}
