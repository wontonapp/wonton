//! 隧道客户端示例。
//!
//! ```text
//! # 打印本机 endpoint ID，把它加进服务端的 --allow
//! cargo run -p wonton-tunnel --example tunnel-client -- id
//! # 用配对码（wonton://pair?…）把本机登记成核心的设备
//! cargo run -p wonton-tunnel --example tunnel-client -- pair '<配对码>'
//! # 在本机起 HTTP 代理：http://127.0.0.1:8081 即服务端的上游 HTTP 服务
//! cargo run -p wonton-tunnel --example tunnel-client -- proxy --server <服务端 endpoint ID>
//! # 像 App 一样起带会话凭据的 HTTP 正向代理（curl -x http://127.0.0.1:8081 -U wonton:<密码>）
//! cargo run -p wonton-tunnel --example tunnel-client -- proxy --server <服务端 endpoint ID> --auth
//! # 测量直连与中继的比例和延迟
//! cargo run -p wonton-tunnel --example tunnel-client -- probe --server <服务端 endpoint ID>
//! ```
//!
//! 使用 n0 的公共中继和地址发现。Ctrl-C 退出。

#[path = "../common/mod.rs"]
mod common;
mod probe;

use std::{net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::Result;
use clap::{Parser, Subcommand};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey, endpoint::presets};
use wonton_tunnel::{LocalProxy, ProxyAccess, ProxyCredential, TunnelClient};

/// Wonton 隧道客户端示例。
#[derive(Debug, Parser)]
struct Cli {
    /// 客户端私钥文件，不存在时自动生成；私钥决定 endpoint ID。默认 `$XDG_STATE_HOME/wonton/tunnel-client.key`
    /// （没设时 `~/.local/state/wonton/tunnel-client.key`），不放当前目录。
    #[arg(long, global = true)]
    key: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 打印本机 endpoint ID。
    Id,
    /// 用配对码把本机登记成核心的设备，打印核心的回应。
    Pair {
        /// 二维码里的文字（`wonton://pair?…`）。
        code: String,
        /// 报给核心的设备名。
        #[arg(long, default_value = "隧道客户端示例")]
        name: String,
    },
    /// 在本机起 HTTP 代理，经隧道转发到服务端的上游 HTTP 服务。
    Proxy {
        #[command(flatten)]
        server: ServerArgs,
        /// 代理监听地址，只能是回环地址。
        #[arg(long, default_value = "127.0.0.1:8081")]
        listen: SocketAddr,
        /// 像 App 一样：生成会话凭据，按 HTTP 正向代理工作，只接受带凭据的请求。不加时不校验调用方，
        /// 把代理当成服务端访问即可，但本机任何进程都能借用这台设备的配对身份。
        #[arg(long)]
        auth: bool,
        /// 加 `--auth` 时只转发这个基础域名和它的子域名，别的地址回 403。
        #[arg(long, default_value = "wonton.localhost")]
        domain: String,
    },
    /// 多轮冷启动建连，测量直连与中继的比例、往返时间和经隧道的 HTTP 延迟。
    Probe {
        #[command(flatten)]
        server: ServerArgs,
        #[command(flatten)]
        probe: probe::ProbeArgs,
    },
}

#[derive(Debug, clap::Args)]
struct ServerArgs {
    /// 服务端 endpoint ID。
    #[arg(long)]
    server: EndpointId,
    /// 服务端的中继地址。不给时靠 n0 的 DNS 地址发现。
    #[arg(long)]
    relay: Option<RelayUrl>,
    /// 服务端的直连地址，可以重复。
    #[arg(long = "addr")]
    addrs: Vec<SocketAddr>,
    /// 只经中继、不走 UDP 直连，用来测打洞失败时的中继延迟。
    #[arg(long)]
    relay_only: bool,
}

impl ServerArgs {
    fn addr(&self) -> EndpointAddr {
        let mut addr = EndpointAddr::new(self.server);
        if let Some(relay) = &self.relay {
            addr = addr.with_relay_url(relay.clone());
        }
        for ip in &self.addrs {
            addr = addr.with_ip_addr(*ip);
        }
        addr
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    common::init_tracing();
    let cli = Cli::parse();
    let key = common::load_or_create_key(&common::key_path(cli.key.clone(), "tunnel-client.key")?)?;
    match cli.command {
        Command::Id => println!("{}", key.public()),
        Command::Pair { code, name } => {
            let code = wonton_tunnel::pair::PairingCode::parse(&code)?;
            let endpoint = bind_endpoint(key, false).await?;
            let reply = wonton_tunnel::pair::pair(&endpoint, &code, &name).await?;
            endpoint.close().await;
            println!("{}", serde_json::to_string(&reply)?);
            if !matches!(reply, wonton_tunnel::pair::PairResponse::Paired) {
                anyhow::bail!("核心拒绝了配对");
            }
        }
        Command::Proxy {
            server,
            listen,
            auth,
            domain,
        } => {
            let access = if auth {
                let credential = ProxyCredential::generate();
                println!(
                    "会话凭据：{}:{}",
                    credential.username(),
                    credential.password()
                );
                ProxyAccess::Credential { credential, domain }
            } else {
                ProxyAccess::Open
            };
            run_proxy(key, &server, listen, access).await?;
        }
        Command::Probe { server, probe } => {
            probe::run(key, server.addr(), server.relay_only, &probe).await?;
        }
    }
    Ok(())
}

/// 客户端 endpoint。`relay_only` 时去掉 UDP 直连传输，只剩中继。
async fn bind_endpoint(key: SecretKey, relay_only: bool) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::N0).secret_key(key);
    if relay_only {
        builder = builder.clear_ip_transports();
    }
    Ok(builder.bind().await?)
}

async fn run_proxy(
    key: SecretKey,
    server: &ServerArgs,
    listen: SocketAddr,
    access: ProxyAccess,
) -> Result<()> {
    let endpoint = bind_endpoint(key, server.relay_only).await?;
    println!("本机 endpoint ID：{}", endpoint.id());
    let client = TunnelClient::new(endpoint.clone(), server.addr());
    let proxy = LocalProxy::bind(client.clone(), listen, access).await?;
    println!("本地代理：http://{}", proxy.local_addr());
    let reporter = tokio::spawn(report(client.clone()));

    tokio::signal::ctrl_c().await?;
    reporter.abort();
    proxy.shutdown().await;
    client.close();
    endpoint.close().await;
    Ok(())
}

/// 打印状态变化；连接可用时每 10 秒打印一次当前路径。
async fn report(client: TunnelClient) {
    let mut status = client.watch_status();
    let mut ticker = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            changed = status.changed() => {
                if changed.is_err() {
                    return;
                }
                println!("状态：{}", *status.borrow_and_update());
            }
            _ = ticker.tick() => {
                if let Some(path) = client.path() {
                    println!("路径：{} {}，RTT {:?}", path.kind, path.remote, path.rtt);
                }
            }
        }
    }
}
