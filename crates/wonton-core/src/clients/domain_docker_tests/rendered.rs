//! 服务商换了，入口配置和插件渲染出来的 compose 都不变。

use super::*;

pub(super) fn compare(case: &Case, rendered: Rendered) -> (Vec<u8>, Vec<u8>) {
    let settings = EntrySettings {
        domain: ZONE.into(),
        service_url: None,
        service_ca: None,
        relay_ca: None,
        registration: None,
        acme_directory: Some("https://pebble.test:14000/dir".into()),
        acme_ca: None,
        acme_resolvers: Vec::new(),
        acme_propagation_disable_ans: true,
        acme_email: None,
        acme_accept_terms: true,
        acme_network: "bridge".into(),
        dns_provider: None,
        dns_env_file: None,
        external_certificate: false,
    };
    let install = case.install.clone();
    let base_compose = "services:\n  core:\n    volumes: []\n    networks:\n      core:\n        aliases: []\n    environment: {}\n  tuwunel:\n    volumes: []\n";
    let compose = crate::entry_compose::render(base_compose, Some(&settings), &install).unwrap();
    // 每次的子网不同：把这条随机地址归一成固定值再比。
    (
        rendered
            .config
            .replace(&case.connector_ip, "connector.test")
            .into_bytes(),
        compose.into_bytes(),
    )
}
