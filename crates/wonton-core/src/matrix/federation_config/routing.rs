//! 联邦代理范围和黑名单：有名字的服务器开放联邦，私用名保留 ADR-0050。

use std::collections::BTreeSet;

use super::{PROXY_ADDR, allowlist_toml, blocks_toml};

/// 只有私用名保留自签证书、配对名单和 TLS 例外（ADR-0051）。
pub fn is_private(server_name: &str) -> bool {
    server_name.ends_with(".wonton.internal")
}

/// 空 include 表示所有域名；有名字且没有配对时，必须整个移除代理规则。
pub fn config_toml(
    server_name: &str,
    paired: &BTreeSet<String>,
    forbidden: &BTreeSet<String>,
) -> String {
    let mut text = String::from("[global]\n");
    if is_private(server_name) {
        text.push_str(&allowlist_toml(paired));
    }
    text.push_str(&blocks_toml(forbidden));
    if !is_private(server_name) && paired.is_empty() {
        text.push_str("proxy = \"none\"\n");
        return text;
    }
    let included = if is_private(server_name) {
        String::new()
    } else {
        paired
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    text.push_str(&format!(
        "\n[global.proxy]\n[[global.proxy.by_domain]]\nurl = \"http://{PROXY_ADDR}\"\ninclude = [{included}]\nexclude = [\"push.wonton.internal\"]\n"
    ));
    text
}

pub(super) fn validate_name(name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !name.is_empty()
            && name.len() <= 253
            && name.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            }),
        "服务器名必须是小写 DNS 名字：{name}"
    );
    Ok(())
}
