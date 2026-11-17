use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use wonton_remote_mcp::{Config, router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listen: SocketAddr = std::env::var("WONTON_REMOTE_MCP_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_owned())
        .parse()?;
    let mut args = std::env::args().skip(1);
    let command = args.next();
    if args.next().is_some() {
        return Err("usage: wonton-remote-mcp [healthcheck]".into());
    }
    if command.as_deref() == Some("healthcheck") {
        let ip = match listen.ip() {
            IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
            ip => ip,
        };
        let response = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()?
            .get(format!(
                "http://{}/healthz",
                SocketAddr::new(ip, listen.port())
            ))
            .send()
            .await?;
        if response.status() != reqwest::StatusCode::NO_CONTENT {
            return Err("connector healthcheck failed".into());
        }
        return Ok(());
    }
    if command.is_some() {
        return Err("usage: wonton-remote-mcp [healthcheck]".into());
    }
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    let config = Config::production()?;
    let listener = tokio::net::TcpListener::bind(listen).await?;
    axum::serve(listener, router(config))
        .with_graceful_shutdown(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                tracing::error!(%error, "could not install shutdown signal handler");
            }
        })
        .await?;
    Ok(())
}
