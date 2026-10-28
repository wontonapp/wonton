//! 连接信息：家里服务器的地址和插件网页入口。
//!
//! M0 由开发脚本给出（`deploy/dev/core/dev.sh ticket`），配对上线后来自二维码（#16）。格式是 JSON：
//!
//! ```json
//! {
//!   "server": {
//!     "endpoint_id": "<64 位十六进制>",
//!     "relay_urls": ["https://…"],
//!     "direct_addrs": ["192.168.0.2:18482"]
//!   },
//!   "base_domain": "wonton.localhost",
//!   "web_url": "http://app-grafana.wonton.localhost:18480/"
//! }
//! ```
//!
//! `server` 和核心的 `GET /_wonton/api/tunnel` 返回的字段相同，`base_domain` 也来自那里：本地代理只转发
//! 这个域名和它的子域名，插件网页入口也必须在它下面。直连地址要写手机能到达的地址，例如局域网
//! IP 加核心发布的隧道端口；n0 的地址发现在国内靠不住（ADR-0004），所以不靠它找服务端。

use std::net::SocketAddr;

use iroh::{EndpointAddr, EndpointId, RelayUrl};
use serde::Deserialize;
use url::Url;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Ticket {
    server: Server,
    base_domain: String,
    #[serde(default)]
    web_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Server {
    endpoint_id: EndpointId,
    #[serde(default)]
    relay_urls: Vec<RelayUrl>,
    #[serde(default)]
    direct_addrs: Vec<SocketAddr>,
}

impl Ticket {
    /// 解析连接信息。至少要有一个中继地址或直连地址：只给 endpoint ID 时要靠地址发现，国内靠不住。
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let ticket: Self =
            serde_json::from_str(text).map_err(|e| format!("连接信息不是合法的 JSON：{e}"))?;
        if ticket.server.relay_urls.is_empty() && ticket.server.direct_addrs.is_empty() {
            return Err("连接信息里没有服务端的中继地址或直连地址".to_owned());
        }
        let domain = ticket
            .base_domain
            .trim_end_matches('.')
            .to_ascii_lowercase();
        let valid_label = |label: &str| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        };
        if !domain.split('.').all(valid_label) {
            return Err(format!("基础域名不合法：{}", ticket.base_domain));
        }
        if let Some(text) = &ticket.web_url {
            let url = Url::parse(text).map_err(|e| format!("插件网页地址不合法：{e}"))?;
            let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
            let in_domain = host == domain || host.ends_with(&format!(".{domain}"));
            if !matches!(url.scheme(), "http" | "https") || !in_domain {
                return Err(format!(
                    "插件网页地址要是 {domain} 下的 http(s) 地址：{text}"
                ));
            }
        }
        Ok(Self {
            base_domain: domain,
            ..ticket
        })
    }

    pub(crate) fn server_id(&self) -> EndpointId {
        self.server.endpoint_id
    }

    pub(crate) fn server_addr(&self) -> EndpointAddr {
        let mut addr = EndpointAddr::new(self.server.endpoint_id);
        for relay in &self.server.relay_urls {
            addr = addr.with_relay_url(relay.clone());
        }
        for ip in &self.server.direct_addrs {
            addr = addr.with_ip_addr(*ip);
        }
        addr
    }

    pub(crate) fn web_url(&self) -> Option<&str> {
        self.web_url.as_deref()
    }

    /// 家里服务器的基础域名（小写，不带末尾的点）。
    pub(crate) fn base_domain(&self) -> &str {
        &self.base_domain
    }
}

#[cfg(test)]
mod tests {
    use iroh::SecretKey;

    use super::*;

    #[test]
    fn tickets_need_an_address_a_domain_and_a_web_url_under_it() {
        let id = SecretKey::generate().public();
        let ok = format!(
            r#"{{"server": {{"endpoint_id": "{id}", "direct_addrs": ["127.0.0.1:18482"]}},
                "base_domain": "Wonton.Localhost",
                "web_url": "http://app-grafana.wonton.localhost:18480/"}}"#
        );
        let ticket = Ticket::parse(&ok).unwrap();
        assert_eq!(ticket.server_id(), id);
        assert_eq!(ticket.server_addr().ip_addrs().count(), 1);
        assert_eq!(ticket.base_domain(), "wonton.localhost");

        let no_address = format!(
            r#"{{"server": {{"endpoint_id": "{id}"}}, "base_domain": "wonton.localhost"}}"#
        );
        let no_domain = ok.replace(r#""base_domain": "Wonton.Localhost","#, "");
        let bad_domain = ok.replace("Wonton.Localhost", "wonton localhost");
        let bad_scheme = ok.replace("http://app-grafana", "javascript://app-grafana");
        let other_site = ok.replace("app-grafana.wonton.localhost", "evil.example");
        for bad in [
            "",
            "{}",
            "not json",
            &no_address,
            &no_domain,
            &bad_domain,
            &bad_scheme,
            &other_site,
        ] {
            assert!(Ticket::parse(bad).is_err(), "{bad}");
        }
    }
}
