//! 连接信息：家里服务器的地址和插件网页入口。
//!
//! 扫码配对成功后由核心按配对码生成（见 `pairing`），测试里经 `ConnectHome` 直接给出。格式是 JSON：
//!
//! ```json
//! {
//!   "server": {
//!     "endpoint_id": "<64 位十六进制>",
//!     "relays": [{"url": "https://…", "token": "…", "tier": 0}],
//!     "direct_addrs": ["192.168.0.2:18482"]
//!   },
//!   "base_domain": "wonton.localhost",
//!   "core_url": "http://wonton.localhost:18480",
//!   "web_url": "http://app-grafana.wonton.localhost:18480/"
//! }
//! ```
//!
//! `core_url` 是核心自己的地址（经隧道访问），App 从这里取 Matrix 登录令牌；没有时用基础域名，协议和端口
//! 跟 `web_url` 一样。
//!
//! `server` 和核心的 `GET /_wonton/api/tunnel` 返回的字段相同，`base_domain` 也来自那里：本地代理只转发
//! 这个域名和它的子域名，插件网页入口也必须在它下面。`relays` 是核心的中继表（ADR-0048），App 用它建
//! 自己一侧的 endpoint。直连地址要写手机能到达的地址，例如局域网 IP 加核心发布的隧道端口；n0 的地址发现
//! 在国内靠不住（ADR-0004），所以不靠它找服务端。

use std::net::SocketAddr;

use iroh::{EndpointAddr, EndpointId};
use serde::{Deserialize, Serialize};
use url::Url;
use wonton_l10n::l10n;
use wonton_tunnel::relays::Relay;

use crate::l10n::{CATALOG, language};

/// 连接信息里最多存这么多直连地址。服务端报的都换成新的，多了只是握手的初始包多发几份。
const MAX_DIRECT_ADDRS: usize = 8;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct Ticket {
    server: Server,
    base_domain: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    core_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    web_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Server {
    endpoint_id: EndpointId,
    #[serde(default)]
    relays: Vec<Relay>,
    #[serde(default)]
    direct_addrs: Vec<SocketAddr>,
}

impl Ticket {
    /// 解析连接信息。至少要有一个中继地址或直连地址：只给 endpoint ID 时要靠地址发现，国内靠不住。
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let ticket: Self = serde_json::from_str(text).map_err(|e| {
            l10n!(
                CATALOG,
                language(),
                "home-ticket-invalid-json",
                error = e.to_string()
            )
        })?;
        if ticket.server.relays.is_empty() && ticket.server.direct_addrs.is_empty() {
            return Err(l10n!(CATALOG, language(), "home-ticket-no-addresses"));
        }
        for relay in &ticket.server.relays {
            relay.validate().map_err(|reason| {
                l10n!(
                    CATALOG,
                    language(),
                    "home-ticket-relays-invalid",
                    reason = reason
                )
            })?;
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
            return Err(l10n!(
                CATALOG,
                language(),
                "home-ticket-invalid-domain",
                domain = ticket.base_domain.as_str()
            ));
        }
        if let Some(text) = &ticket.web_url {
            let url = Url::parse(text).map_err(|e| {
                l10n!(
                    CATALOG,
                    language(),
                    "home-ticket-invalid-web-url",
                    error = e.to_string()
                )
            })?;
            let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
            let in_domain = host == domain || host.ends_with(&format!(".{domain}"));
            if !matches!(url.scheme(), "http" | "https") || !in_domain {
                return Err(l10n!(
                    CATALOG,
                    language(),
                    "home-ticket-web-url-under-domain",
                    domain = domain.as_str(),
                    url = text.as_str()
                ));
            }
        }
        if let Some(text) = &ticket.core_url {
            let url = Url::parse(text).map_err(|e| {
                l10n!(
                    CATALOG,
                    language(),
                    "home-ticket-invalid-core-url",
                    error = e.to_string()
                )
            })?;
            let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
            if !matches!(url.scheme(), "http" | "https") || host != domain {
                return Err(l10n!(
                    CATALOG,
                    language(),
                    "home-ticket-core-url-under-domain",
                    domain = domain.as_str(),
                    url = text.as_str()
                ));
            }
        }
        Ok(Self {
            base_domain: domain,
            ..ticket
        })
    }

    /// 核心自己的地址。没给时用基础域名，协议和端口跟插件网页地址一样（都由核心的同一个入口提供）。
    pub(crate) fn core_url(&self) -> Url {
        if let Some(url) = self
            .core_url
            .as_deref()
            .and_then(|text| Url::parse(text).ok())
        {
            return url;
        }
        let mut url = Url::parse(&format!("http://{}/", self.base_domain))
            .expect("validated base domain always forms a URL");
        if let Some(web) = self
            .web_url
            .as_deref()
            .and_then(|text| Url::parse(text).ok())
        {
            url.set_scheme(web.scheme()).ok();
            url.set_port(web.port()).ok();
        }
        url
    }

    pub(crate) fn server_id(&self) -> EndpointId {
        self.server.endpoint_id
    }

    pub(crate) fn server_addr(&self) -> EndpointAddr {
        let mut addr = EndpointAddr::new(self.server.endpoint_id);
        for relay in &self.server.relays {
            addr = addr.with_relay_url(relay.url.clone());
        }
        for ip in &self.server.direct_addrs {
            addr = addr.with_ip_addr(*ip);
        }
        addr
    }

    /// 连接信息里的中继表，App 建 endpoint 用。
    pub(crate) fn relays(&self) -> &[Relay] {
        &self.server.relays
    }

    /// 记上核心报告的地址；和已有的一样时返回 `None`。中继表或直连地址报告为空时留着旧的：中继可能
    /// 只是暂时没连上，地址也可能只是这一次没测出来。令牌变了以后，跑着的 endpoint 不会跟着换
    /// （iroh#4642），要等下一次重建 endpoint。
    ///
    /// 直连地址是替换，不和旧的合并：核心报的就是手机到得了的地址（ADR-0032），旧地址只会越积越多，
    /// 容器网卡地址永远清不掉。
    pub(crate) fn with_reported(
        &self,
        relays: Vec<Relay>,
        direct_addrs: &[SocketAddr],
    ) -> Option<Self> {
        let mut updated = self.clone();
        if !relays.is_empty() {
            updated.server.relays = relays;
        }
        if !direct_addrs.is_empty() {
            updated.server.direct_addrs = direct_addrs
                .iter()
                .copied()
                .take(MAX_DIRECT_ADDRS)
                .collect();
        }
        let changed = updated.server.relays != self.server.relays
            || updated.server.direct_addrs != self.server.direct_addrs;
        changed.then_some(updated)
    }

    /// 码里给的服务端直连地址。
    pub(crate) fn direct_addrs(&self) -> &[SocketAddr] {
        &self.server.direct_addrs
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
    use wonton_tunnel::relays::RelayToken;

    use super::*;

    #[test]
    fn reported_addresses_replace_the_old_ones() {
        let id = SecretKey::generate().public();
        let text = format!(
            r#"{{"server": {{"endpoint_id": "{id}", "direct_addrs": ["192.168.1.2:18482"]}},
                "base_domain": "wonton.localhost", "web_url": "http://wonton.localhost:1/"}}"#
        );
        let ticket = Ticket::parse(&text).unwrap();
        let addr = |n: u8| SocketAddr::from(([10, 0, 0, n], 18482));

        assert!(
            ticket
                .with_reported(Vec::new(), &ticket.server.direct_addrs)
                .is_none()
        );
        // 报来的为空：留着旧的。
        assert!(ticket.with_reported(Vec::new(), &[]).is_none());

        let moved = ticket.with_reported(Vec::new(), &[addr(1)]).unwrap();
        assert_eq!(moved.direct_addrs(), [addr(1)]);
        let saved = serde_json::to_string(&moved).unwrap();
        let again = Ticket::parse(&saved).unwrap();
        assert_eq!(again.direct_addrs(), moved.direct_addrs());
        assert_eq!(again.web_url(), moved.web_url());

        let many: Vec<SocketAddr> = (1..=9).map(addr).collect();
        let full = moved.with_reported(Vec::new(), &many).unwrap();
        assert_eq!(full.direct_addrs(), &many[..8]);
    }

    #[test]
    fn reported_relays_replace_the_saved_table_and_an_empty_report_keeps_the_old_one() {
        let id = SecretKey::generate().public();
        let text = format!(
            r#"{{"server": {{"endpoint_id": "{id}", "relays": [{{"url": "https://a.example/", "tier": 0}}], "direct_addrs": ["192.168.1.2:18482"]}},
                "base_domain": "wonton.localhost"}}"#
        );
        let ticket = Ticket::parse(&text).unwrap();
        assert_eq!(ticket.relays().len(), 1);
        assert_eq!(
            ticket
                .server_addr()
                .relay_urls()
                .cloned()
                .collect::<Vec<_>>(),
            ["https://a.example/".parse().unwrap()]
        );
        assert!(ticket.with_reported(Vec::new(), &[]).is_none());
        let updated = ticket
            .with_reported(
                vec![Relay {
                    url: "https://b.example/".parse().unwrap(),
                    token: Some(RelayToken::new("tok")),
                    tier: 0,
                }],
                &[],
            )
            .unwrap();
        assert_eq!(updated.relays()[0].url.as_str(), "https://b.example/");
        assert_eq!(updated.relays()[0].token.as_ref().unwrap().expose(), "tok");
        // 地址里藏令牌的表项不被接受：令牌单独放。
        let smuggled = format!(
            r#"{{"server": {{"endpoint_id": "{id}", "relays": [{{"url": "https://tok@a.example/", "tier": 0}}]}},
                "base_domain": "wonton.localhost"}}"#
        );
        assert!(Ticket::parse(&smuggled).is_err());
    }

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
        // 没给核心地址时，用基础域名加插件网页的协议和端口。
        assert_eq!(ticket.core_url().as_str(), "http://wonton.localhost:18480/");
        let explicit = ok.replace(
            r#""base_domain""#,
            r#""core_url": "https://wonton.localhost", "base_domain""#,
        );
        assert_eq!(
            Ticket::parse(&explicit).unwrap().core_url().as_str(),
            "https://wonton.localhost/"
        );
        let foreign_core = ok.replace(
            r#""base_domain""#,
            r#""core_url": "http://evil.example", "base_domain""#,
        );
        assert!(Ticket::parse(&foreign_core).is_err());

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
