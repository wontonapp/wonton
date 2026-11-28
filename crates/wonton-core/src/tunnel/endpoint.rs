use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::str::FromStr;

use anyhow::Context as _;
use iroh::endpoint::presets;
use iroh::tls::CaTlsConfig;
use iroh::{Endpoint, RelayMode, SecretKey};
use rustls_pki_types::{CertificateDer, pem::PemObject};
use wonton_tunnel::relays::{FALLBACK_AFTER, RelayTable};

use crate::secret;

/// 读取或生成核心的隧道私钥。私钥决定服务端的 endpoint ID，换了它所有设备都要重新配对，所以要持久化。
pub fn load_or_create_key(path: &Path) -> anyhow::Result<SecretKey> {
    let hex =
        secret::load_or_create(path).with_context(|| format!("读取隧道私钥 {}", path.display()))?;
    SecretKey::from_str(&hex).with_context(|| format!("隧道私钥 {} 已损坏", path.display()))
}

/// 绑定隧道 endpoint。中继表从配置来（ADR-0048），一开始只放最优先的一档，掉档的回退交给
/// [`wonton_tunnel::relays::spawn_tier_fallback`]；`port` 为 0 时由系统分配，固定端口便于防火墙放行
/// 和容器发布端口。
/// 中继用测试 CA 签证书时（开发栈），把 `SSL_CERT_FILE` 里的根也交给 iroh 校验；默认仍用内置 Mozilla 根。
fn relay_roots() -> Option<Vec<CertificateDer<'static>>> {
    let path = std::env::var("SSL_CERT_FILE").ok()?;
    let data = std::fs::read(path).ok()?;
    let roots: Vec<_> = CertificateDer::pem_slice_iter(&data)
        .filter_map(Result::ok)
        .collect();
    (!roots.is_empty()).then_some(roots)
}

fn rustls_pemfile_roots(
    pem: &[u8],
) -> anyhow::Result<Vec<rustls_pki_types::CertificateDer<'static>>> {
    let mut roots = Vec::new();
    for item in rustls_pemfile::certs(&mut std::io::Cursor::new(pem)) {
        roots.push(item.context("读取 PEM 证书")?);
    }
    Ok(roots)
}

pub async fn bind(
    key: SecretKey,
    port: u16,
    table: &RelayTable,
) -> anyhow::Result<(Endpoint, wonton_tunnel::relays::TierFallbackHandle)> {
    let mut builder = Endpoint::builder(presets::N0)
        .secret_key(key)
        .relay_mode(RelayMode::Custom(wonton_tunnel::relays::relay_map(
            &table.views(),
            1,
        )));
    if let Some(roots) = relay_roots() {
        builder = builder.ca_tls_config(iroh::tls::CaTlsConfig::default().with_extra_roots(roots));
    }
    if let Some(host) = std::env::var("WONTON_TUNNEL_BIND")
        .ok()
        .filter(|s| !s.is_empty())
    {
        let address = wonton_tunnel::bind_address::resolve_bind_v4(&host, port).await?;
        builder = builder.clear_ip_transports().bind_addr(address)?;
    } else if port != 0 {
        builder = builder
            .bind_addr((Ipv4Addr::UNSPECIFIED, port))?
            .bind_addr((Ipv6Addr::UNSPECIFIED, port))?;
    }
    // iroh 默认只信编译进去的根；冒烟和私有部署用 WONTON_SHARE_CA_FILE 显式加一份。
    if let Some(path) = std::env::var_os("WONTON_SHARE_CA_FILE") {
        let pem = std::fs::read(path).context("读取隧道额外的根证书")?;
        let roots = rustls_pemfile_roots(&pem).context("解析隧道额外的根证书")?;
        builder = builder.ca_tls_config(CaTlsConfig::custom_roots(roots));
    }
    let endpoint = builder.bind().await.context("绑定隧道 endpoint")?;
    let fallback = wonton_tunnel::relays::spawn_tier_fallback(
        endpoint.clone(),
        table.clone(),
        FALLBACK_AFTER,
        None,
    );
    Ok((endpoint, fallback))
}
