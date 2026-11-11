//! A real iroh relay with a locally ACME-issued certificate; never disables verification.
use anyhow::{Context, Result};
use iroh_relay::server::{CertConfig, QuicConfig, RelayConfig, Server, ServerConfig, TlsConfig};
use std::net::{Ipv4Addr, SocketAddr};
use tokio_rustls::rustls::{
    self,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cert = args
        .get(1)
        .context("usage: share-smoke-relay <fullchain.pem> <privkey.pem>")?;
    let key = args.get(2).context("missing private key")?;
    let chain = CertificateDer::pem_file_iter(cert)?.collect::<Result<Vec<_>, _>>()?;
    let key = PrivateKeyDer::from_pem_file(key)?;
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let tls = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(provider))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(chain, key)?;
    let mut relay = RelayConfig::new(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 80)));
    relay.tls = Some(TlsConfig::new(
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, 443)),
        CertConfig::Manual { server_config: tls },
    ));
    let mut config = ServerConfig::default();
    // 数据面只有 QUIC；TLS 从 relay.tls 继承，同一个 443（UDP）。
    config.quic = Some(QuicConfig::new(SocketAddr::from((
        Ipv4Addr::UNSPECIFIED,
        443,
    ))));
    config.relay = Some(relay);
    let server = Server::spawn(config).await?;
    println!("share smoke relay ready https://relay.test/");
    tokio::signal::ctrl_c().await?;
    drop(server);
    Ok(())
}
