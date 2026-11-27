//! 配对联邦的证书材料（ADR-0112）：核心在 iroh 隧道两端终结 TLS，只认经配对钉住的对端。
//!
//! 装机时生成，全部落在 `secrets/peer-tls/`（目录 0700、密钥 0600）、已有的成对保留：
//!
//! - `ca.pem`/`ca.key`：本机私有 CA，只给核心对本机 Tuwunel 出示的证书签名。那张证书按 CONNECT 里的目标
//!   服务器名现签（SAN 要和对端名字一致），所以这里只存 CA，不存固定的叶。CA 私钥能签任意名字，对外只钉叶。
//! - `peer.pem`/`peer.key`：对外出示的自签叶，SAN 是本机服务器名；配对时交换的就是这张叶，pin 就是它。
//! - `trust.pem`：系统根证书加本机 CA，只读挂给 Tuwunel（[`TUWUNEL_DIR`]）当 `SSL_CERT_FILE`；核心现签的
//!   证书由它链到本机 CA。
//! - `system-roots.pem`/`system-roots.source`：从当前镜像抄来的系统根和来源记录；镜像里的根变了就重抄。
//!
//! 信任文件只放这两类：只信本机 CA 会让 Tuwunel 连不了别的服务器，关掉校验则谁都认。

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use time::{Duration, OffsetDateTime};

use crate::install::Report;

/// 材料在 `secrets/` 下的子目录。
pub(crate) const DIR: &str = "peer-tls";
/// Tuwunel 容器里挂信任文件的目录（`SSL_CERT_FILE` 指向它下面的 [`TRUST`]）。
pub(crate) const TUWUNEL_DIR: &str = "/etc/wonton-peer-tls";
pub(crate) const CA_CERT: &str = "ca.pem";
pub(crate) const CA_KEY: &str = "ca.key";
pub(crate) const PEER_CERT: &str = "peer.pem";
pub(crate) const PEER_KEY: &str = "peer.key";
pub(crate) const TRUST: &str = "trust.pem";
const ROOTS: &str = "system-roots.pem";
const ROOTS_SOURCE: &str = "system-roots.source";
const CA_NAME: &str = "Wonton peer federation CA";
/// 证书有效期（ADR-0112 §5）。
const VALID_DAYS: i64 = 5 * 365;

pub(crate) fn dir(secrets: &Path) -> PathBuf {
    secrets.join(DIR)
}

/// 系统根的旧内容和来源记录。
struct Roots {
    pem: String,
    source: String,
}

fn material(secrets: &Path) -> anyhow::Result<PathBuf> {
    let dir = dir(secrets);
    fs::create_dir_all(&dir).with_context(|| format!("建立 {}", dir.display()))?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("设置 {} 的权限", dir.display()))?;
    Ok(dir)
}

/// 需要时生成材料、刷新系统根和信任包，返回材料目录。已有的密钥一概不动；`image` 只进来源记录。
pub(crate) fn ensure(
    secrets: &Path,
    server_name: &str,
    image: Option<&str>,
    report: &mut Report,
) -> anyhow::Result<PathBuf> {
    let dir = material(secrets)?;
    let ca = kept_or_generated(&dir, CA_CERT, CA_KEY, report, || {
        let key = KeyPair::generate().context("生成配对用的 CA 私钥")?;
        let cert = ca_params()?.self_signed(&key).context("签配对用的 CA")?;
        Ok((cert.pem(), key.serialize_pem()))
    })?;
    outward(&dir, secrets, server_name, report)?;

    let loaded = loaded_roots()?;
    let roots = match fs::read_to_string(dir.join(ROOTS)) {
        Ok(pem) => Some(Roots {
            pem,
            source: fs::read_to_string(dir.join(ROOTS_SOURCE)).unwrap_or_default(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("读取 {}", dir.join(ROOTS).display())),
    };
    let source = format!("{}\n", image.unwrap_or("unknown"));
    let refresh = match &roots {
        None => true,
        Some(roots) => roots.pem != loaded,
    };
    let roots = if refresh {
        let path = dir.join(ROOTS);
        let existed = path.exists();
        crate::install::write_public(&path, &loaded)?;
        if existed {
            report.kept.push(path);
        } else {
            report.created.push(path);
        }
        let path = dir.join(ROOTS_SOURCE);
        crate::install::write_public(&path, &source)?;
        Roots {
            pem: loaded,
            source,
        }
    } else {
        let mut roots = roots.expect("没有时上面判了刷新");
        let path = dir.join(ROOTS);
        report.kept.push(path);
        if roots.source != source {
            let path = dir.join(ROOTS_SOURCE);
            crate::install::write_public(&path, &source)?;
            roots.source = source;
        }
        roots
    };
    let trust = dir.join(TRUST);
    let bundle = format!("{}{}", roots.pem, ca);
    let existed = trust.exists();
    crate::install::write_public(&trust, &bundle)?;
    if existed {
        report.kept.push(trust);
    } else {
        report.created.push(trust);
    }
    // 材料写完后确认 CA 私钥和证书能一起读成签发者（隧道出站按 CONNECT 的名字现签都走这里）：现在坏了
    // 就报，别等第一次联邦出站才发现。
    let _ = ca_issuer(&dir)?;
    Ok(dir)
}

/// 对外出示的证书。升级前的私用名安装把它的自签证书放在 Tuwunel 目录（更早的在 `secrets/` 根上），
/// 挪过来接着用：老对端 pin 的就是这张叶，重新生成会让它们握不上手。
fn outward(
    dir: &Path,
    secrets: &Path,
    server_name: &str,
    report: &mut Report,
) -> anyhow::Result<()> {
    let cert = dir.join(PEER_CERT);
    let key = dir.join(PEER_KEY);
    if cert.exists() && key.exists() {
        report.kept.push(cert);
        report.kept.push(key);
        return Ok(());
    }
    let legacy = [
        secrets.join("tuwunel/tuwunel-tls.crt"),
        secrets.join("tuwunel-tls.crt"),
    ];
    for old_cert in &legacy {
        let old_key = old_cert_key(old_cert);
        if !old_cert.exists() || !old_key.exists() {
            continue;
        }
        fs::rename(old_cert, &cert).with_context(|| format!("挪 {}", old_cert.display()))?;
        fs::rename(&old_key, &key).with_context(|| format!("挪 {}", old_key.display()))?;
        // 另一处还留着的旧文件读不到了，删掉免得两处分叉。
        for stale in &legacy {
            for path in [stale.clone(), old_cert_key(stale)] {
                let _ = fs::remove_file(path);
            }
        }
        report.kept.push(cert);
        report.kept.push(key);
        return Ok(());
    }
    let key = dir.join(PEER_KEY);
    let pair = KeyPair::generate().context("生成对外的证书私钥")?;
    let cert_pem = leaf_params(server_name)?
        .self_signed(&pair)
        .with_context(|| format!("给 {server_name} 签对外证书"))?
        .pem();
    crate::install::write_private(&cert, &cert_pem)?;
    crate::install::write_private(&key, &pair.serialize_pem())?;
    report.created.push(cert);
    report.created.push(key);
    Ok(())
}

fn old_cert_key(cert: &Path) -> PathBuf {
    cert.with_extension("key")
}

/// 本机 CA 的签发者，给 CONNECT 的目标名现签证书用。CA 证书按同一组参数重建：验证只看名字和公钥。
pub(crate) fn ca_issuer(dir: &Path) -> anyhow::Result<Issuer<'static, KeyPair>> {
    let pem = fs::read_to_string(dir.join(CA_KEY)).context("读本机 CA 私钥")?;
    let key = KeyPair::from_pem(&pem).context("解析本机 CA 私钥")?;
    Ok(Issuer::new(ca_params()?, key))
}

/// 一起读或一起没有；只有一半时按没有处理，重新生成一对。
fn kept_or_generated(
    dir: &Path,
    cert_name: &str,
    key_name: &str,
    report: &mut Report,
    create: impl FnOnce() -> anyhow::Result<(String, String)>,
) -> anyhow::Result<String> {
    let (cert, key) = (dir.join(cert_name), dir.join(key_name));
    if cert.exists() && key.exists() {
        report.kept.push(cert.clone());
        report.kept.push(key);
        return fs::read_to_string(&cert).with_context(|| format!("读取 {}", cert.display()));
    }
    let (cert_pem, key_pem) = create()?;
    crate::install::write_private(&cert, &cert_pem)?;
    crate::install::write_private(&key, &key_pem)?;
    report.created.push(cert);
    report.created.push(key);
    Ok(cert_pem)
}

fn validity() -> (OffsetDateTime, OffsetDateTime) {
    let now = OffsetDateTime::now_utc();
    (now - Duration::days(1), now + Duration::days(VALID_DAYS))
}

fn ca_params() -> anyhow::Result<CertificateParams> {
    let mut params = CertificateParams::default();
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, CA_NAME);
    params.distinguished_name = name;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let (from, to) = validity();
    params.not_before = from;
    params.not_after = to;
    Ok(params)
}

/// 给一个 CONNECT 目标名签证书用的参数：SAN 就是它，五年、只作服务端。签发者用 [`ca_issuer`]。
pub(crate) fn leaf_params(server_name: &str) -> anyhow::Result<CertificateParams> {
    let mut params = CertificateParams::new(vec![server_name.to_owned()])
        .with_context(|| format!("服务器名 {server_name} 不能写进证书"))?;
    params
        .distinguished_name
        .push(DnType::CommonName, server_name);
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let (from, to) = validity();
    params.not_before = from;
    params.not_after = to;
    Ok(params)
}

/// 当前镜像里的系统根，PEM。读不到一张就报错：信任包里只剩本机 CA，Tuwunel 就验不了别人的证书。
fn loaded_roots() -> anyhow::Result<String> {
    let loaded = rustls_native_certs::load_native_certs();
    if loaded.certs.is_empty() {
        let errors: Vec<String> = loaded.errors.iter().map(ToString::to_string).collect();
        bail!("读不到系统根证书：{}", errors.join("；"));
    }
    Ok(loaded
        .certs
        .iter()
        .map(|cert| pem("CERTIFICATE", cert.as_ref()))
        .collect())
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
    use rcgen::date_time_ymd;

    use super::*;

    /// 有效期从今天算五年，不是日期写死的。
    #[test]
    fn leaves_are_valid_for_five_years() {
        let (from, to) = validity();
        // notBefore 比现在早一天，整段比五年多一天。
        assert_eq!((to - from).whole_days(), VALID_DAYS + 1);
        assert!(from < OffsetDateTime::now_utc());
        // rcgen 的日期和 time 的类型是同一个，年份跟着现在走。
        assert_eq!(date_time_ymd(2000, 1, 1).year(), 2000);
    }
}
