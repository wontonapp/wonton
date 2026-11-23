//! 出站联邦的 TLS 材料（ADR-0112）：用装机生成的本机 CA 给 CONNECT 的目标名现签证书。
//!
//! 本机 Tuwunel 按 CONNECT 里的对方名校验证书，但核心手里没有对方的私钥，所以不能出示固定的叶；每次
//! CONNECT 都按那个名字现签一张（SAN 就是它），链里带上 CA。Tuwunel 的 `SSL_CERT_FILE` 是系统根加这个
//! CA 的 `trust.pem`，链就能验到。
//!
//! 对端的校验材料来自配对记录（`peers`）；旧记录没有 pin 时返回 `None`，握手按“待钉”只校名字（ADR-0112
//! §5b）。

use std::{path::Path, sync::Arc};

use anyhow::Context as _;
use rcgen::KeyPair;
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use tokio_rustls::TlsAcceptor;
use wonton_tunnel::federation::{ForwarderTls, PeerPinLookup};

use crate::peer_tls;

/// 出站转发器要的 TLS 材料。`dir` 是 `WONTON_PEER_TLS_DIR`（装机生成的 `secrets/peer-tls`）。
pub fn forwarder(dir: &Path, peer_pin: PeerPinLookup) -> anyhow::Result<ForwarderTls> {
    for name in [peer_tls::CA_CERT, peer_tls::CA_KEY] {
        let path = dir.join(name);
        anyhow::ensure!(path.is_file(), "配对联邦的证书材料缺少 {}", path.display());
    }
    let dir = dir.to_owned();
    Ok(ForwarderTls {
        local_certificates: Arc::new(move |name| acceptor(&dir, name)),
        peer_pin,
    })
}

/// 给一个 CONNECT 目标名签一张服务端证书，链里带上本机 CA。
fn acceptor(dir: &Path, name: &str) -> anyhow::Result<TlsAcceptor> {
    let issuer = peer_tls::ca_issuer(dir)?;
    let params = peer_tls::leaf_params(name)?;
    let key = KeyPair::generate().context("生成本机 Tuwunel 的证书私钥")?;
    let leaf = params.signed_by(&key, &issuer)?;
    let ca_path = dir.join(peer_tls::CA_CERT);
    let ca_pem =
        std::fs::read(&ca_path).with_context(|| format!("读本机 CA 证书 {}", ca_path.display()))?;
    let ca: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut ca_pem.as_slice())
        .collect::<Result<_, _>>()
        .context("解析本机 CA 证书")?;
    anyhow::ensure!(!ca.is_empty(), "本机 CA 文件里没有证书");
    let mut chain = vec![leaf.der().clone()];
    chain.extend(ca);
    let key = PrivatePkcs8KeyDer::from(key.serialize_der()).into();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(chain, key)?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rustls::pki_types::ServerName;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::TlsConnector;

    use super::*;

    /// 本机 Tuwunel 拿到的证书按 CONNECT 的对方名现签、链到本机 CA，名字校得过。
    #[tokio::test]
    async fn signs_a_certificate_for_the_name_the_local_tuwunel_asks_for() {
        let root = tempfile::tempdir().unwrap();
        let secrets = root.path().join("secrets");
        let mut report = crate::install::Report::default();
        peer_tls::ensure(&secrets, "home.example", None, &mut report).unwrap();
        let dir = peer_tls::dir(&secrets);
        let acceptor = acceptor(&dir, "friend.example").unwrap();

        let mut roots = rustls::RootCertStore::empty();
        let ca = std::fs::read(dir.join(peer_tls::CA_CERT)).unwrap();
        for cert in rustls_pemfile::certs(&mut ca.as_slice()) {
            roots.add(cert.unwrap()).unwrap();
        }
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();

        let (server, client) = tokio::io::duplex(4096);
        let server = tokio::spawn(async move { acceptor.accept(server).await.unwrap() });
        let mut tls = TlsConnector::from(Arc::new(config))
            .connect(
                ServerName::try_from("friend.example".to_owned()).unwrap(),
                client,
            )
            .await
            .unwrap();
        tls.write_all(b"hi").await.unwrap();
        let mut server = server.await.unwrap();
        let mut buf = [0u8; 2];
        server.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hi");
    }
}
