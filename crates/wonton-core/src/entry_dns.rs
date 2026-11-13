//! ACME 下单前独立查询 DNS，不能把管理接口的成功当成 CAA 已生效。

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, ensure};
use hickory_resolver::config::{NameServerConfig, ResolverConfig};
use hickory_resolver::net::DnsError;
use hickory_resolver::net::NetError;
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::proto::rr::{RData, RecordType};
use hickory_resolver::{Resolver, TokioResolver};

use wonton_protocol::CaaState;

/// 一条 CAA 的 `issue` 记录：签发的 CA 和它的参数（`accounturi`、`validationmethods` 等）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaaIssue {
    /// `issue` 或 `issuewild`。
    pub tag: String,
    pub issuer: Option<String>,
    pub parameters: Vec<(String, String)>,
}

/// `<domain>` 上的 CAA。没有记录时是空列表，查询本身失败才报错。
pub async fn caa_issues(domain: &str, servers: &[String]) -> anyhow::Result<Vec<CaaIssue>> {
    let resolver = resolver(servers)?;
    let records = match resolver.lookup(format!("{domain}."), RecordType::CAA).await {
        Ok(records) => records,
        Err(NetError::Dns(DnsError::NoRecordsFound(_))) => return Ok(Vec::new()),
        Err(error) => return Err(error).context("查询 CAA 失败"),
    };
    Ok(records
        .answers()
        .iter()
        .filter_map(|record| {
            let RData::CAA(caa) = &record.data else {
                return None;
            };
            let (issuer, parameters) = caa.value_as_issue().ok()?;
            Some(CaaIssue {
                tag: caa.tag.clone(),
                issuer: issuer.map(|name| name.to_string().trim_end_matches('.').to_owned()),
                parameters: parameters
                    .iter()
                    .map(|p| (p.key().to_ascii_lowercase(), p.value().to_owned()))
                    .collect(),
            })
        })
        .collect())
}

/// 把查到的 CAA 归成设置页显示的状态。只有整条 `issue` 都限定到本机账号（`accounturi` 相等、
/// `validationmethods` 含 `dns-01`）才算 [`CaaState::Bound`]；有 CAA 但都不是就是 [`CaaState::Mismatch`]。
pub fn caa_state(records: &[CaaIssue], account_uri: &str) -> CaaState {
    if records.is_empty() {
        return CaaState::Missing;
    }
    let bound = records.iter().any(|record| {
        // 状态页问的是后缀（apex），只有 `issue` 管它；`issuewild` 只管通配符。
        record.tag.eq_ignore_ascii_case("issue")
            && record
                .parameters
                .iter()
                .any(|(key, value)| key.eq_ignore_ascii_case("accounturi") && value == account_uri)
            && record.parameters.iter().any(|(key, value)| {
                // RFC 8657 的列表是逗号分隔；方法名按 Boulder 的语义精确比较，只认小写 dns-01
                // （Boulder va/caa.go 对方法名不做大小写归一，大写 DNS-01 实际会被拒单）。
                key.eq_ignore_ascii_case("validationmethods")
                    && value.split(',').any(|method| method.trim() == "dns-01")
            })
    });
    if bound {
        CaaState::Bound
    } else {
        CaaState::Mismatch
    }
}

/// 家名服务路径的强校验：没绑定就报错、不下单。
pub async fn verify_caa(domain: &str, account_uri: &str, servers: &[String]) -> anyhow::Result<()> {
    let records = caa_issues(domain, servers).await.context("未签发证书")?;
    ensure!(
        caa_state(&records, account_uri) == CaaState::Bound,
        "DNS 中的 CAA 尚未绑定本机 ACME 账号；未签发证书"
    );
    Ok(())
}

fn resolver(servers: &[String]) -> anyhow::Result<TokioResolver> {
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
    builder.build().context("建立 DNS 查询器")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(pairs: &[(&str, &str)]) -> CaaIssue {
        CaaIssue {
            tag: "issue".into(),
            issuer: Some("letsencrypt.org".into()),
            parameters: pairs
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into()))
                .collect(),
        }
    }

    #[test]
    fn bound_needs_the_exact_account_and_dns_01() {
        let record = issue(&[
            ("accounturi", "https://pebble.test/my-account/1"),
            ("validationmethods", "dns-01"),
        ]);
        assert_eq!(
            caa_state(
                std::slice::from_ref(&record),
                "https://pebble.test/my-account/1"
            ),
            CaaState::Bound
        );
        assert_eq!(
            caa_state(&[record], "https://pebble.test/my-account/2"),
            CaaState::Mismatch
        );
    }

    async fn wire_issues(tag: &str, methods: &str) -> Vec<CaaIssue> {
        use hickory_resolver::proto::op::{Message, OpCode};
        use hickory_resolver::proto::rr::{Record, rdata::CAA};
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let server = socket.local_addr().unwrap().to_string();
        let mut caa = CAA::new_issue(false, None, Vec::new());
        caa.tag = tag.into();
        caa.value = format!(
            "letsencrypt.org; accounturi=https://pebble.test/my-account/1; validationmethods={methods}"
        ).into_bytes();
        let responding = tokio::spawn(async move {
            let mut bytes = [0; 2048];
            let (length, peer) = socket.recv_from(&mut bytes).await.unwrap();
            let query = Message::from_vec(&bytes[..length]).unwrap();
            let question = query.queries[0].clone();
            let mut reply = Message::response(query.metadata.id, OpCode::Query);
            reply.queries.push(question.clone());
            reply
                .answers
                .push(Record::from_rdata(question.name, 60, RData::CAA(caa)));
            socket
                .send_to(&reply.to_vec().unwrap(), peer)
                .await
                .unwrap();
        });
        let records = caa_issues("example.test", &[server]).await.unwrap();
        responding.await.unwrap();
        records
    }

    #[tokio::test]
    async fn validation_methods_match_boulder_case_sensitively() {
        let account = "https://pebble.test/my-account/1";
        for methods in ["DNS-01", "http-01,DNS-01", "Dns-01"] {
            let records = wire_issues("issue", methods).await;
            assert_eq!(records.len(), 1, "CAA RDATA was not parsed: {methods}");
            assert_eq!(
                caa_state(&records, account),
                CaaState::Mismatch,
                "{methods}"
            );
        }
    }

    #[tokio::test]
    async fn method_lists_and_issuewild() {
        let account = "https://pebble.test/my-account/1";
        for methods in ["dns-01", "http-01,dns-01", "dns-01,http-01"] {
            let records = wire_issues("issue", methods).await;
            assert_eq!(records.len(), 1, "CAA RDATA was not parsed: {methods}");
            assert_eq!(caa_state(&records, account), CaaState::Bound, "{methods}");
        }
        let records = wire_issues("issuewild", "dns-01").await;
        assert_eq!(records.len(), 1);
        assert_eq!(caa_state(&records, account), CaaState::Mismatch);
    }

    #[test]
    fn none_is_missing_and_other_policies_are_mismatch() {
        assert_eq!(
            caa_state(&[], "https://pebble.test/my-account/1"),
            CaaState::Missing
        );
        let without_dns = issue(&[("accounturi", "https://pebble.test/my-account/1")]);
        assert_eq!(
            caa_state(&[without_dns], "https://pebble.test/my-account/1"),
            CaaState::Mismatch
        );
        let other_ca = CaaIssue {
            tag: "issue".into(),
            issuer: Some("other-ca.example".into()),
            parameters: Vec::new(),
        };
        assert_eq!(
            caa_state(&[other_ca], "https://pebble.test/my-account/1"),
            CaaState::Mismatch
        );
    }
}
