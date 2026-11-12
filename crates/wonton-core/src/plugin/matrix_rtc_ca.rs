//! `matrix_rtc` 服务器名别名的证书（ADR-0096）：核心自己的 CA 给服务器名签证书，提供方信任“系统根证书 + 这个 CA”
//! 合成的证书包。只信核心的 CA 会让授权服务验不了别的服务器来的用户；跳过校验则谁的证书都认。
//!
//! CA 的私钥存在核心的状态目录里，重启后不变：提供方的容器挂着的证书包不用跟着核心重启换。证书包里的 CA 证书
//! 每次按同一把私钥、同一个名字现签，验证只看名字和公钥，和上次签的那张等价。

use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

use super::matrix_rtc::server_host;

/// 状态目录里 CA 私钥的文件名。
const KEY_FILE: &str = "matrix-rtc-ca.key";
/// 提供方数据目录下放证书包的子目录，经 [`CA_DIR_VAR`] 交给插件。
pub const CA_DIR: &str = "matrix-rtc-ca";
/// 证书包的文件名：PEM，系统根证书在前，核心的 CA 在最后。
pub const BUNDLE_FILE: &str = "ca-bundle.pem";
/// 注入给 `matrix_rtc` 提供方的变量：放证书包的目录（宿主路径）。
pub const CA_DIR_VAR: &str = "WONTON_MATRIX_CA_DIR";
const CA_NAME: &str = "Wonton core matrix_rtc CA";

/// 核心的 CA：签服务器名的证书，给出提供方要信任的证书包。
pub struct CoreCa {
    issuer: Issuer<'static, KeyPair>,
    cert_pem: String,
}

impl std::fmt::Debug for CoreCa {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreCa").finish_non_exhaustive()
    }
}

impl CoreCa {
    /// 读 `state_dir` 里的 CA 私钥，没有就生成一把存下（只有核心能读）。
    pub fn load_or_create(state_dir: &Path) -> anyhow::Result<Arc<Self>> {
        let path = state_dir.join(KEY_FILE);
        let key = match std::fs::read_to_string(&path) {
            Ok(pem) => KeyPair::from_pem(&pem)
                .with_context(|| format!("读 matrix_rtc 的 CA 私钥 {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let key = KeyPair::generate().context("生成 matrix_rtc 的 CA 私钥")?;
                write_private(&path, key.serialize_pem().as_bytes())?;
                key
            }
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("读 matrix_rtc 的 CA 私钥 {}", path.display()));
            }
        };
        Self::from_key(key).map(Arc::new)
    }

    /// 测试用：不落盘的 CA。
    #[cfg(test)]
    pub fn ephemeral() -> Arc<Self> {
        Arc::new(Self::from_key(KeyPair::generate().unwrap()).unwrap())
    }

    fn from_key(key: KeyPair) -> anyhow::Result<Self> {
        let mut params = CertificateParams::default();
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, CA_NAME);
        params.distinguished_name = name;
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let cert_pem = params
            .self_signed(&key)
            .context("签 matrix_rtc 的 CA 证书")?
            .pem();
        Ok(Self {
            issuer: Issuer::new(params, key),
            cert_pem,
        })
    }

    /// CA 证书（PEM）。
    pub fn cert_pem(&self) -> &str {
        &self.cert_pem
    }

    /// 给服务器名（去掉端口）签一张证书，交给别名的 TLS 监听。
    pub fn acceptor(&self, server_name: &str) -> anyhow::Result<TlsAcceptor> {
        let key = KeyPair::generate().context("生成别名证书的私钥")?;
        let mut params = CertificateParams::new(vec![server_host(server_name).to_owned()])
            .with_context(|| format!("服务器名 {server_name} 不能写进证书"))?;
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let leaf = params
            .signed_by(&key, &self.issuer)
            .with_context(|| format!("给 {server_name} 签证书"))?;
        let chain = vec![CertificateDer::from(leaf.der().to_vec())];
        let key = PrivatePkcs8KeyDer::from(key.serialize_der()).into();
        let provider = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider();
        let config = ServerConfig::builder_with_provider(Arc::new(provider))
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(chain, key)?;
        Ok(TlsAcceptor::from(Arc::new(config)))
    }

    /// 核心看到的系统根证书加上这个 CA。系统里一张根证书也读不到时报错：只剩核心的 CA，提供方就验不了别的服务器。
    pub fn bundle(&self) -> anyhow::Result<String> {
        let loaded = rustls_native_certs::load_native_certs();
        if loaded.certs.is_empty() {
            let errors: Vec<String> = loaded.errors.iter().map(ToString::to_string).collect();
            anyhow::bail!("核心读不到系统根证书：{}", errors.join("；"));
        }
        let mut bundle = String::new();
        for cert in &loaded.certs {
            bundle.push_str(&pem("CERTIFICATE", cert.as_ref()));
        }
        bundle.push_str(&self.cert_pem);
        Ok(bundle)
    }

    /// 把证书包写进提供方的 `<数据目录>/`[`CA_DIR`]`/`[`BUNDLE_FILE`]，返回目录。先写临时文件再改名：容器挂的是
    /// 目录，读到的总是完整的一份。
    pub fn write_bundle(&self, app_data: &Path) -> anyhow::Result<PathBuf> {
        let dir = app_data.join(CA_DIR);
        std::fs::create_dir_all(&dir).with_context(|| format!("建目录 {}", dir.display()))?;
        let bundle = self.bundle()?;
        let tmp = dir.join(format!(".{BUNDLE_FILE}.tmp"));
        std::fs::write(&tmp, bundle).with_context(|| format!("写 {}", tmp.display()))?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
        std::fs::rename(&tmp, dir.join(BUNDLE_FILE))
            .with_context(|| format!("换上 {}", dir.join(BUNDLE_FILE).display()))?;
        Ok(dir)
    }
}

fn write_private(path: &Path, content: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("写 matrix_rtc 的 CA 私钥 {}", path.display()))?;
    file.write_all(content)?;
    file.sync_all()?;
    Ok(())
}

fn pem(label: &str, der: &[u8]) -> String {
    let encoded = STANDARD.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for line in encoded.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).expect("base64 是 ASCII"));
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_survives_a_restart_and_the_bundle_keeps_the_system_roots() {
        let dir = tempfile::tempdir().unwrap();
        let first = CoreCa::load_or_create(dir.path()).unwrap();
        let saved = std::fs::read(dir.path().join(KEY_FILE)).unwrap();
        CoreCa::load_or_create(dir.path()).unwrap();
        assert_eq!(std::fs::read(dir.path().join(KEY_FILE)).unwrap(), saved);
        let mode = std::fs::metadata(dir.path().join(KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);

        let app = tempfile::tempdir().unwrap();
        let written = first.write_bundle(app.path()).unwrap();
        let bundle = std::fs::read_to_string(written.join(BUNDLE_FILE)).unwrap();
        assert!(bundle.ends_with(first.cert_pem()));
        assert!(
            bundle.matches("BEGIN CERTIFICATE").count() > 1,
            "系统根证书也在里面"
        );
    }
}
