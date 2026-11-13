//! `wonton-core init <安装目录> <基础域名>`：正式安装时由安装脚本在核心镜像里执行（`deploy/install/`），生成各组件的
//! 密钥和 Tuwunel 的配置，写出 compose 文件。
//!
//! - 已有的密钥一律不动，重复执行（重装、升级）不会让已配对的设备和 Matrix 账号失效。
//! - compose 文件和 Tuwunel 的配置由代码决定，每次都按当前版本重写；Matrix 的 `server_name` 第一次定下后记在
//!   `secrets/matrix-server-name`，之后不能改，要的和记下的不一致时拒绝（ADR-0009 的“Matrix 的服务器名”）。
//! - 密钥在 `secrets/`（0700，文件 0600），主机上不需要 openssl。

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::matrix::federation_config;
use crate::secret;
use anyhow::{Context, anyhow, bail};
use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use wonton_tunnel::relays::RelayTable;

/// 正式安装的 compose 文件，随核心的版本走。
pub const COMPOSE: &str = include_str!("../../../deploy/install/compose.yaml");
/// 主机上的 `wonton` 命令（`sudo wonton pair`），也随核心的版本走：升级时跟着换。
pub const CLI: &str = include_str!("../../../deploy/install/wonton");

/// 写出或保留的文件。
#[derive(Debug, Default)]
pub struct Report {
    pub created: Vec<PathBuf>,
    pub kept: Vec<PathBuf>,
}

/// 写出已选定身份的配置；CLI 在调用前完成家名注册和证书准备，`None` 仅用于明确的私用模式。
pub fn init(dir: &Path, requested: Option<&str>) -> anyhow::Result<Report> {
    if let Some(name) = requested {
        validate_server_name(name)?;
    }
    // 安装脚本把主人的中继表传进来（ADR-0048），不合法时当场报错，不用等核心容器反复重启。
    if let Some(text) = std::env::var("WONTON_RELAYS")
        .ok()
        .filter(|value| !value.is_empty())
    {
        RelayTable::parse(&text).map_err(|reason| anyhow!("WONTON_RELAYS 不合法：{reason}"))?;
    }
    let secrets = dir.join("secrets");
    fs::create_dir_all(&secrets).with_context(|| format!("建立 {}", secrets.display()))?;
    fs::set_permissions(&secrets, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("设置 {} 的权限", secrets.display()))?;
    // Tuwunel 只挂这个子目录：同目录下的 `matrix-jwt.pem`、`hydra-secret` 是核心和 Hydra 的东西，
    // 联邦容器读不到才对（ADR-0051）。
    let tuwenel = secrets.join("tuwunel");
    fs::create_dir_all(&tuwenel).with_context(|| format!("建立 {}", tuwenel.display()))?;
    fs::set_permissions(&tuwenel, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("设置 {} 的权限", tuwenel.display()))?;
    let mut report = Report::default();

    let key_path = secrets.join("matrix-jwt.pem");
    let public_key = matrix_key(&key_path, &mut report)?;
    let registration = kept_or_created(&secrets.join("matrix-registration-secret"), &mut report)?;
    let hydra = kept_or_created(&secrets.join("hydra-secret"), &mut report)?;
    write_private(
        &secrets.join("hydra.env"),
        &format!("SECRETS_SYSTEM={hydra}\n"),
    )?;

    let config = tuwenel.join("tuwunel.toml");
    let server_name = server_name(&secrets, &config, requested, &mut report)?;
    if federation_config::is_private(&server_name) {
        tls_cert(&tuwenel, &server_name, &mut report)?;
    }
    write_private(
        &config,
        &tuwunel_config(&server_name, &registration, &public_key),
    )?;
    fed_files(&tuwenel, &server_name, &mut report)?;
    // 这份 PR 之前配置和名单在 `secrets/` 根和 `fed/` 下；现在都不挂了，删掉免得两处内容分叉。
    for stale in [secrets.join("tuwunel.toml"), dir.join("fed/config.toml")] {
        match fs::remove_file(&stale) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("删除 {}", stale.display())),
        }
    }
    let settings = crate::entry_settings::EntrySettings::load(dir)?;
    write_private(
        &dir.join("compose.yaml"),
        &crate::entry_compose::render(COMPOSE, settings.as_ref(), dir)?,
    )?;
    write_file(&dir.join("wonton"), CLI, 0o755)?;
    Ok(report)
}

/// 记下的服务器名优先。没记下而已有 Tuwunel 的配置，是记录之前装的服务器：Tuwunel 的数据库里全是那个名字，照旧用它，
/// 换名字要重装 Matrix 的数据。都没有时用指定的，没指定就随机生成。
fn server_name(
    secrets: &Path,
    tuwenel: &Path,
    requested: Option<&str>,
    report: &mut Report,
) -> anyhow::Result<String> {
    let record = secrets.join("matrix-server-name");
    let existing = match fs::read_to_string(&record) {
        Ok(text) => {
            report.kept.push(record.clone());
            Some(text.trim().to_owned())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match read_server_name(tuwenel)? {
            Some(name) => Some(name),
            // 这份 PR 之前装在 `secrets/tuwunel.toml`，升级时照旧认它。
            None => read_server_name(&secrets.join("tuwunel.toml"))?,
        },
        Err(e) => return Err(e).with_context(|| format!("读取 {}", record.display())),
    };
    let name = match (existing, requested) {
        (Some(existing), Some(requested)) if existing != requested => bail!(
            "已有的 Matrix 服务器名是 {existing}，不能改成 {requested}：账号的用户 ID 都带着它"
        ),
        (Some(existing), _) => existing,
        (None, Some(requested)) => requested.to_owned(),
        (None, None) => random_server_name(),
    };
    validate_server_name(&name)
        .with_context(|| format!("{} 里的服务器名不对", record.display()))?;
    if !record.exists() {
        write_private(&record, &format!("{name}\n"))?;
        report.created.push(record);
    }
    Ok(name)
}

/// 120 位随机数的小写 base32 加 `.wonton.internal`：每台不一样，不带主人的任何信息，也不是隧道的 endpoint ID，
/// 换隧道密钥不牵连它。`.internal` 是 ICANN 留给私用的顶级域，不会被公网注册。
fn random_server_name() -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut bytes = [0u8; 15];
    getrandom::fill(&mut bytes).expect("操作系统随机数源不可用");
    let bits = bytes.iter().fold(0u128, |acc, b| acc << 8 | u128::from(*b));
    let label: String = (0..24)
        .rev()
        .map(|i| char::from(ALPHABET[(bits >> (i * 5)) as usize & 31]))
        .collect();
    format!("{label}.wonton.internal")
}

/// 和核心的 `WONTON_BASE_DOMAIN` 一样的规则：小写字母、数字和连字符组成的点分标签。
pub(crate) fn validate_server_name(name: &str) -> anyhow::Result<()> {
    let valid = !name.is_empty()
        && name.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        });
    if !valid {
        bail!("Matrix 服务器名 {name} 不合法：只能用小写字母、数字、连字符和点");
    }
    Ok(())
}

/// 读出或生成核心签发 Matrix 登录用的 Ed25519 私钥（PKCS#8 PEM），返回给 Tuwunel 验签的公钥（SPKI PEM）。
fn matrix_key(path: &Path, report: &mut Report) -> anyhow::Result<String> {
    let pair = match fs::read_to_string(path) {
        Ok(pem) => {
            report.kept.push(path.to_owned());
            let der = pem_body(&pem).with_context(|| format!("{} 不是 PEM", path.display()))?;
            Ed25519KeyPair::from_pkcs8(&der)
                .map_err(|e| anyhow::anyhow!("{} 不是 Ed25519 私钥：{e}", path.display()))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let pair = Ed25519KeyPair::generate()
                .map_err(|e| anyhow::anyhow!("生成 Ed25519 密钥失败：{e}"))?;
            let pkcs8 = pair
                .to_pkcs8()
                .map_err(|e| anyhow::anyhow!("导出 Ed25519 私钥失败：{e}"))?;
            write_private(path, &pem("PRIVATE KEY", pkcs8.as_ref()))?;
            report.created.push(path.to_owned());
            pair
        }
        Err(e) => return Err(e).with_context(|| format!("读取 {}", path.display())),
    };
    let public = pair
        .public_key()
        .as_der()
        .map_err(|e| anyhow::anyhow!("导出 Ed25519 公钥失败：{e}"))?;
    Ok(pem("PUBLIC KEY", public.as_ref()))
}

fn kept_or_created(path: &Path, report: &mut Report) -> anyhow::Result<String> {
    let existed = path.exists();
    let value =
        secret::load_or_create(path).with_context(|| format!("读取或生成 {}", path.display()))?;
    if existed {
        report.kept.push(path.to_owned());
    } else {
        report.created.push(path.to_owned());
    }
    Ok(value)
}

fn pem(label: &str, der: &[u8]) -> String {
    let body = STANDARD.encode(der);
    let lines: Vec<&str> = body
        .as_bytes()
        .chunks(64)
        .map(|chunk| std::str::from_utf8(chunk).expect("base64 都是 ASCII"))
        .collect();
    format!(
        "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
        lines.join("\n")
    )
}

fn pem_body(text: &str) -> anyhow::Result<Vec<u8>> {
    let body: String = text
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    Ok(STANDARD.decode(body.trim())?)
}

fn read_server_name(path: &Path) -> anyhow::Result<Option<String>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("读取 {}", path.display())),
    };
    Ok(text.lines().find_map(|line| {
        line.strip_prefix("server_name = \"")
            .and_then(|rest| rest.strip_suffix('"'))
            .map(str::to_owned)
    }))
}

/// 证书校验按安装时定下的模式选择；配对目标和黑名单在第二份配置里热加载。
fn tuwunel_config(server_name: &str, registration_secret: &str, public_key: &str) -> String {
    let private = federation_config::is_private(server_name);
    let ports = if private { "[8008, 8448]" } else { "8008" };
    let tls = if private {
        "\n[global.tls]\ncerts = \"/etc/tuwunel/tuwunel-tls.crt\"\nkey = \"/etc/tuwunel/tuwunel-tls.key\"\ndual_protocol = true\n"
    } else {
        ""
    };
    format!(
        r#"# 由 wonton-core init 生成，升级时按新版本重写；改了会被覆盖。
[global]
server_name = "{server_name}"
database_path = "/var/lib/tuwunel"
address = "0.0.0.0"
# 有名字时 TLS 在隔离入口容器里终止；私用模式保留内部 8448。
port = {ports}
allow_federation = true
trusted_servers = []
# 配对代理是内部地址；公开直连的目的地仍由 Matrix 认证。
ip_range_denylist = []
# 账号只由核心签发：设备和 Bot 用 JWT 登录，首次登录自动建号。不开放注册，也不接受密码登录。
allow_registration = false
login_with_password = false
new_user_displayname_suffix = ""
# 核心用它注册自己的管理账号（Synapse 兼容的管理接口）。
registration_shared_secret = "{registration_secret}"
# 私用名拿不到公共证书，仅封闭配对模式保留此例外（ADR-0051）。
allow_invalid_tls_certificates = {private}

[global.jwt]
enable = true
# 核心持有 Ed25519 私钥，Tuwunel 只拿公钥验签。
format = "EDDSA"
algorithm = "EdDSA"
require_exp = true
# 用户由核心签的 JWT 第一次登录时建（核心没有单独的建号调用）；写明而不靠 Tuwunel 的默认值。
register_user = true
issuer = ["wonton-core"]
audience = ["tuwunel"]
key = """
{public_key}"""

{tls}
"#
    )
}

/// 为本机服务器名生成自签证书，放进 Tuwunel 的配置目录（0600）。已有的成对保留：重装、升级不换。
/// 这份 PR 之前它和 `tuwunel.toml` 一起放在 `secrets/` 根上，升级时挪进子目录。
fn tls_cert(tuwenel: &Path, server_name: &str, report: &mut Report) -> anyhow::Result<()> {
    let cert = tuwenel.join("tuwunel-tls.crt");
    let key = tuwenel.join("tuwunel-tls.key");
    if cert.exists() && key.exists() {
        report.kept.push(cert);
        report.kept.push(key);
        return Ok(());
    }
    let old = tuwenel.parent().context("secrets 没有上级目录")?;
    let (old_cert, old_key) = (old.join("tuwunel-tls.crt"), old.join("tuwunel-tls.key"));
    if old_cert.exists() && old_key.exists() {
        fs::rename(&old_cert, &cert)?;
        fs::rename(&old_key, &key)?;
        report.kept.push(cert);
        report.kept.push(key);
        return Ok(());
    }
    let pair = rcgen::generate_simple_self_signed(vec![server_name.to_owned()])
        .map_err(|e| anyhow::anyhow!("生成 {server_name} 的自签证书失败：{e}"))?;
    write_private(&cert, &pair.cert.pem())?;
    write_private(&key, &pair.signing_key.serialize_pem())?;
    report.created.push(cert);
    report.created.push(key);
    Ok(())
}

/// 联邦名单的初值（`secrets/tuwunel/wonton-fed.toml`，ADR-0050）。只在还没有时创建：第一次
/// `docker compose up` 把它挂进 Tuwunel 需要文件已经存在，之后的内容由核心运行时原地重写（不换 inode——
/// 容器可能还开着旧文件）。
fn fed_files(tuwenel: &Path, server_name: &str, report: &mut Report) -> anyhow::Result<()> {
    let path = tuwenel.join("wonton-fed.toml");
    if path.exists() {
        report.kept.push(path);
        return Ok(());
    }
    write_private(
        &path,
        &federation_config::config_toml(server_name, &BTreeSet::new(), &BTreeSet::new()),
    )?;
    report.created.push(path);
    Ok(())
}

pub(crate) fn write_private(path: &Path, text: &str) -> anyhow::Result<()> {
    write_file(path, text, 0o600)
}

/// 先写临时文件再改名：中途失败不会留下写了一半的文件。
fn write_file(path: &Path, text: &str, mode: u32) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(&tmp)
        .with_context(|| format!("写入 {}", tmp.display()))?;
    // 上次中途失败留下的临时文件权限可能不对，`mode` 只在新建时生效。
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(&tmp, path).with_context(|| format!("写入 {}", path.display()))
}

#[cfg(test)]
mod tuwunel_tests;

#[cfg(test)]
mod tests {
    use jsonwebtoken::{Algorithm, DecodingKey, Validation};

    use super::*;
    use crate::matrix::issuer::Issuer;

    #[test]
    fn init_creates_secrets_once_and_keeps_them_on_rerun() {
        let dir = tempfile::tempdir().unwrap();
        let first = init(dir.path(), Some("home.wonton.internal")).unwrap();
        assert!(
            first
                .created
                .contains(&dir.path().join("secrets/matrix-jwt.pem"))
        );
        let key = fs::read_to_string(dir.path().join("secrets/matrix-jwt.pem")).unwrap();
        let config = fs::read_to_string(dir.path().join("secrets/tuwunel/tuwunel.toml")).unwrap();
        let mode = |path: &str| {
            fs::metadata(dir.path().join(path))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode("secrets"), 0o700);
        assert_eq!(mode("secrets/matrix-jwt.pem"), 0o600);
        assert_eq!(mode("secrets/tuwunel"), 0o700);
        assert_eq!(mode("secrets/tuwunel/tuwunel.toml"), 0o600);
        assert_eq!(
            mode("secrets/tuwunel/tuwunel-tls.crt"),
            0o600,
            "联邦的自签证书"
        );
        assert_eq!(
            mode("secrets/tuwunel/tuwunel-tls.key"),
            0o600,
            "联邦的证书私钥"
        );
        assert_eq!(mode("secrets/tuwunel/wonton-fed.toml"), 0o600);
        assert_eq!(mode("wonton"), 0o755, "主机上的 wonton 命令要能执行");

        let again = init(dir.path(), None).unwrap();
        assert!(again.created.is_empty(), "{again:?}");
        assert_eq!(
            fs::read_to_string(dir.path().join("secrets/matrix-jwt.pem")).unwrap(),
            key
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("secrets/tuwunel/tuwunel.toml")).unwrap(),
            config
        );
        assert!(
            init(dir.path(), Some("home.wonton.internal")).is_ok(),
            "重装不换证书和名单文件"
        );
        assert!(
            init(dir.path(), Some("other.example")).is_err(),
            "server_name 不能改"
        );
    }

    fn recorded(dir: &Path) -> String {
        fs::read_to_string(dir.join("secrets/matrix-server-name"))
            .unwrap()
            .trim()
            .to_owned()
    }

    #[test]
    fn without_a_requested_name_each_install_gets_its_own_random_one() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        init(a.path(), None).unwrap();
        init(b.path(), None).unwrap();
        let name = recorded(a.path());
        assert_ne!(name, recorded(b.path()));
        let label = name.strip_suffix(".wonton.internal").unwrap();
        assert_eq!(label.len(), 24, "{name}");
        assert!(
            label
                .bytes()
                .all(|c| matches!(c, b'a'..=b'z' | b'2'..=b'7'))
        );

        init(a.path(), None).unwrap();
        assert_eq!(recorded(a.path()), name, "再执行一次不换名字");
    }

    /// 记录之前装的服务器：Tuwunel 的数据库里全是原来的名字，接着用它。
    /// 这份 PR 之前 `tuwunel.toml` 和证书就放在 `secrets/` 根上，升级时既认旧位置也把文件挪进子目录。
    #[test]
    fn an_install_from_before_the_record_keeps_its_name_and_moves_its_files() {
        let dir = tempfile::tempdir().unwrap();
        // 私用名才有自签证书，三种文件都得跟着搬。
        init(dir.path(), Some("home.wonton.internal")).unwrap();
        let secrets = dir.path().join("secrets");
        fs::remove_file(secrets.join("matrix-server-name")).unwrap();
        for name in ["tuwunel.toml", "tuwunel-tls.crt", "tuwunel-tls.key"] {
            fs::rename(secrets.join("tuwunel").join(name), secrets.join(name)).unwrap();
        }
        let report = init(dir.path(), None).unwrap();
        assert_eq!(recorded(dir.path()), "home.wonton.internal");
        assert!(
            report
                .created
                .contains(&dir.path().join("secrets/matrix-server-name"))
        );
        assert!(!secrets.join("tuwunel.toml").exists(), "旧位置的配置挪走");
        for name in ["tuwunel.toml", "tuwunel-tls.crt", "tuwunel-tls.key"] {
            assert!(
                !secrets.join(name).exists(),
                "旧位置的 {name} 还在，容器读不到"
            );
            assert!(
                secrets.join("tuwunel").join(name).exists(),
                "{name} 挪进子目录"
            );
        }
        assert!(init(dir.path(), Some("home.example")).is_err());
    }

    /// 核心用私钥签的登录令牌，Tuwunel 拿配置里的公钥验得过：两边是同一对密钥。
    #[test]
    fn the_public_key_in_the_tuwunel_config_verifies_the_core_tokens() {
        let dir = tempfile::tempdir().unwrap();
        init(dir.path(), None).unwrap();
        let private = fs::read_to_string(dir.path().join("secrets/matrix-jwt.pem")).unwrap();
        let config = fs::read_to_string(dir.path().join("secrets/tuwunel/tuwunel.toml")).unwrap();
        let public = config
            .split("key = \"\"\"\n")
            .nth(1)
            .and_then(|rest| rest.split("\"\"\"").next())
            .unwrap();
        let token = Issuer::from_pem(&private).unwrap().login_token("owner");
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_audience(&["tuwunel"]);
        let key = DecodingKey::from_ed_pem(public.as_bytes()).unwrap();
        assert!(jsonwebtoken::decode::<serde_json::Value>(&token, &key, &validation).is_ok());
    }

    #[test]
    fn bad_server_names_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["", "Home.example", "a..b", "home_example"] {
            assert!(init(dir.path(), Some(name)).is_err(), "{name}");
        }
    }
}
