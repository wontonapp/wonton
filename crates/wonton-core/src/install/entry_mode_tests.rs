//! 有名字的安装按“配没配入口”分成两种落盘配置（ADR-0090）：有入口时 TLS 全交给入口容器，Tuwunel 只留 8008；
//! 没有入口时公网联邦关着，Tuwunel 也不自签，但 iroh 配对过的对方经隧道投进来的流仍要落在它的联邦端口上。
//!
//! 私用名（`.wonton.internal`）另有一套：仍然保留联邦端口，但同样不自己听 TLS，证书材料和信任文件在
//! `install/peer_tls_tests`。

use std::fs;
use std::path::Path;

use serde_json::Value;

use super::*;
use crate::entry_settings::EntrySettings;

/// 关掉公网入口的落盘设置：只留入口容器要的域名，签发和连接器字段都不参与安装时的这两个分支。
pub(super) fn named_settings(dir: &Path) {
    fs::create_dir_all(dir.join("entry")).unwrap();
    let settings = EntrySettings {
        domain: "home.example".to_owned(),
        service_url: None,
        service_ca: None,
        relay_ca: None,
        registration: None,
        acme_directory: Some("https://acme.example/directory".to_owned()),
        acme_ca: None,
        acme_resolvers: Vec::new(),
        acme_propagation_disable_ans: false,
        acme_email: None,
        acme_accept_terms: false,
        acme_network: "bridge".to_owned(),
        dns_provider: None,
        dns_env_file: None,
        external_certificate: false,
    };
    fs::write(
        dir.join("entry/settings.json"),
        serde_json::to_string(&settings).unwrap(),
    )
    .unwrap();
}

fn config(dir: &Path) -> String {
    fs::read_to_string(dir.join("secrets/tuwunel/tuwunel.toml")).unwrap()
}

fn core_env(dir: &Path) -> Value {
    let compose: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("compose.yaml")).unwrap()).unwrap();
    compose["services"]["core"]["environment"].clone()
}

fn no_self_signed_cert(dir: &Path, report: &Report) {
    let secrets = dir.join("secrets");
    for name in ["tuwunel-tls.crt", "tuwunel-tls.key"] {
        assert!(
            !secrets.join("tuwunel").join(name).exists(),
            "{name} 不该生成"
        );
        assert!(!secrets.join(name).exists(), "{name} 在旧位置也不该有");
        assert!(
            !report.created.contains(&secrets.join("tuwunel").join(name)),
            "{name} 出现在生成列表里"
        );
    }
}

/// 有入口设置：入口容器用有效证书终结 TLS，Tuwunel 不自己听 8448、也不用验证对方证书；
/// iroh 的配对流交给入口的配对前端（8449）。
#[test]
fn a_named_install_with_an_entry_leaves_tls_to_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    named_settings(dir.path());
    let report = init(dir.path(), Some("home.example")).unwrap();

    let config = config(dir.path());
    assert!(config.contains("port = 8008"), "{config}");
    assert!(!config.contains("[global.tls]"), "{config}");
    assert!(
        config.contains("allow_invalid_tls_certificates = false"),
        "{config}"
    );
    no_self_signed_cert(dir.path(), &report);
    assert_eq!(
        core_env(dir.path())["WONTON_MATRIX_FEDERATION_ADDR"],
        "entry.wonton.internal:8449"
    );
}

/// 没有入口设置：公网 8448 不开放、也不自签；Tuwunel 的联邦端口照旧留着，核心的默认投放地址
/// （`tuwunel.wonton.internal:8448`）才落在监听上，而不用指向入口的地址。
#[test]
fn a_named_install_without_an_entry_keeps_the_federation_port_for_the_tunnel() {
    let dir = tempfile::tempdir().unwrap();
    let report = init(dir.path(), Some("home.example")).unwrap();

    let config = config(dir.path());
    assert!(config.contains("port = [8008, 8448]"), "{config}");
    assert!(!config.contains("[global.tls]"), "{config}");
    assert!(
        config.contains("allow_invalid_tls_certificates = false"),
        "{config}"
    );
    no_self_signed_cert(dir.path(), &report);
    let env = core_env(dir.path());
    assert_eq!(
        env["WONTON_MATRIX_FEDERATION_ADDR"], "tuwunel.wonton.internal:8448",
        "联邦流要落在 Tuwunel 的联邦端口上，不指向入口"
    );
}

/// 私用名同样不自己听 TLS：两个端口都在，核心在隧道两端终结，Tuwunel 只和核心说话（ADR-0112）。
#[test]
fn a_private_name_keeps_the_federation_port_without_its_own_tls() {
    let dir = tempfile::tempdir().unwrap();
    let report = init(dir.path(), Some("home.wonton.internal")).unwrap();

    let config = config(dir.path());
    assert!(config.contains("port = [8008, 8448]"), "{config}");
    assert!(!config.contains("[global.tls]"), "{config}");
    assert!(
        config.contains("allow_invalid_tls_certificates = false"),
        "{config}"
    );
    no_self_signed_cert(dir.path(), &report);
    assert_eq!(
        core_env(dir.path())["WONTON_MATRIX_FEDERATION_ADDR"],
        "tuwunel.wonton.internal:8448"
    );
}
