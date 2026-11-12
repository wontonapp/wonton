//! 隧道绑定探针：Docker 行为测试（wonton-core 的 `tests/docker_tunnel_bind.rs`）和入口绑定的实测用。
//! `listen` 按 [`wonton_tunnel::bind_address::resolve_bind_v4`] 解析 `WONTON_TUNNEL_BIND` 只绑那个地址
//! （没设就绑 0.0.0.0），收到的包原样回给发送方，收发的地址都打到 stdout；`send` 发一个包等回包。

use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tokio::net::UdpSocket;

#[derive(Debug, Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 绑定 UDP 端口并回显收到的包。
    Listen {
        /// 绑定的端口。
        port: u16,
    },
    /// 起一个最小隧道服务端：按 `WONTON_TUNNEL_BIND` 绑定（同一规则），允许列表里的设备的每条流
    /// 回一段固定的 HTTP 响应，让 `tunnel-client probe` 能量直连和中继。
    Serve {
        /// 允许的设备 endpoint ID，逗号分隔。
        allow: String,
        /// 隧道端口。
        port: u16,
    },
    /// 发一个 UDP 包，等到回包或超时。
    Send {
        /// 发往的 `地址:端口`。
        target: String,
        /// 包的内容。
        payload: String,
        /// 等回包的秒数，超过退出 1。
        #[arg(long, default_value_t = 3)]
        timeout_secs: u64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Listen { port } => listen(port).await,
        Command::Serve { allow, port } => serve(allow, port).await,
        Command::Send {
            target,
            payload,
            timeout_secs,
        } => send(target, payload, timeout_secs).await,
    }
}

/// 从环境解析绑定的 IPv4 地址，规则和 [`wonton_tunnel::bind_address::resolve_bind_v4`] 的调用方一致：
/// 设了 `WONTON_TUNNEL_BIND` 就只绑解析出的那个地址，没设绑通配。
async fn bind_addr(port: u16) -> std::net::SocketAddr {
    match std::env::var("WONTON_TUNNEL_BIND") {
        Ok(host) if !host.is_empty() => {
            let address = wonton_tunnel::bind_address::resolve_bind_v4(&host, port)
                .await
                .expect("WONTON_TUNNEL_BIND 解析不出 IPv4");
            println!("bound {address}");
            address
        }
        _ => {
            println!("bound 0.0.0.0:{port}");
            std::net::SocketAddr::from(([0, 0, 0, 0], port))
        }
    }
}

async fn listen(port: u16) -> Result<()> {
    let bind = bind_addr(port).await;
    let socket = UdpSocket::bind(bind).await?;
    let mut buffer = [0u8; 2048];
    loop {
        let (len, from) = socket.recv_from(&mut buffer).await?;
        println!(
            "got {} from {from}",
            String::from_utf8_lossy(&buffer[..len])
        );
        socket.send_to(&buffer[..len], from).await?;
    }
}

async fn serve(allow: String, port: u16) -> Result<()> {
    use std::str::FromStr as _;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let _bind = bind_addr(port).await;
    let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
        .secret_key(iroh::SecretKey::generate())
        .clear_ip_transports()
        .bind_addr(_bind)?
        .bind()
        .await?;
    println!("endpoint {}", endpoint.id());
    let mut ids = Vec::new();
    for id in allow.split(',') {
        ids.push(iroh::EndpointId::from_str(id.trim())?);
    }
    let (_server, mut listener) =
        wonton_tunnel::TunnelServer::spawn_embedded(endpoint, wonton_tunnel::Allowlist::new(ids));
    // 一条流一个 HTTP 请求：读到请求头结束，回 200，结束。probe 只看状态码和路径。
    while let Some(mut stream) = listener.accept().await {
        println!("stream from {}", stream.remote_id());
        tokio::spawn(async move {
            let mut buffer = vec![0u8; 8192];
            let mut read = 0;
            loop {
                let len = stream.read(&mut buffer[read..]).await.unwrap_or(0);
                if len == 0 || buffer[..read + len].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
                read += len;
            }
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await;
            let _ = stream.shutdown().await;
        });
    }
    Ok(())
}

async fn send(target: String, payload: String, timeout_secs: u64) -> Result<()> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).await?;
    // UDP 的 connect 不发包，只让内核按目的地过滤回包；解析失败在这里报出来。
    socket
        .connect(&target)
        .await
        .with_context(|| format!("解析或连 {target} 失败"))?;
    socket.send(payload.as_bytes()).await?;
    let mut buffer = [0u8; 2048];
    let reply = tokio::time::timeout(Duration::from_secs(timeout_secs), socket.recv(&mut buffer))
        .await
        .context("超时，没有回包")?
        .context("收回包失败")?;
    println!("{}", String::from_utf8_lossy(&buffer[..reply]));
    Ok(())
}
