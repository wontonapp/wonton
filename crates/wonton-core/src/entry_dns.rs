//! ACME 下单前独立查询 DNS，不能把管理接口的成功当成 CAA 已生效。

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, ensure};
use hickory_resolver::config::{NameServerConfig, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::proto::rr::{RData, RecordType};
use hickory_resolver::{Resolver, TokioResolver};

pub async fn verify_caa(domain: &str, account_uri: &str, servers: &[String]) -> anyhow::Result<()> {
    let mut builder = if servers.is_empty() {
        TokioResolver::builder_tokio().context("读取 DNS 配置")?
    } else {
        let mut nameservers = Vec::new();
        for server in servers {
            let address: SocketAddr = server
                .parse()
                .with_context(|| format!("DNS 解析器必须是 IP:端口：{server}"))?;
            let mut config = NameServerConfig::udp_and_tcp(address.ip());
            for connection in &mut config.connections {
                connection.port = address.port();
            }
            nameservers.push(config);
        }
        Resolver::builder_with_config(
            ResolverConfig::from_name_servers(nameservers),
            TokioRuntimeProvider::default(),
        )
    };
    builder.options_mut().attempts = 1;
    builder.options_mut().timeout = Duration::from_secs(5);
    builder.options_mut().cache_size = 0;
    let resolver = builder.build()?;
    let records = resolver
        .lookup(format!("{domain}."), RecordType::CAA)
        .await
        .context("查询 CAA 失败；未签发证书")?;
    let matched = records.answers().iter().any(|record| {
        let RData::CAA(caa) = &record.data else {
            return false;
        };
        let Ok((Some(_issuer), parameters)) = caa.value_as_issue() else {
            return false;
        };
        parameters
            .iter()
            .any(|p| p.key().eq_ignore_ascii_case("accounturi") && p.value() == account_uri)
            && parameters
                .iter()
                .any(|p| p.key().eq_ignore_ascii_case("validationmethods") && p.value() == "dns-01")
    });
    ensure!(matched, "DNS 中的 CAA 尚未绑定本机 ACME 账号；未签发证书");
    Ok(())
}
