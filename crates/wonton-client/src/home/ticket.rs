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
use wonton_tunnel::relays::{Relay, canonical_relay};

use crate::l10n::{CATALOG, language};

/// 连接信息里最多存这么多核心报来的直连候选。
const MAX_DIRECT_ADDRS: usize = 8;
/// 最多留这么多本机实际连通过的直连地址。历史跨网络保留：换回上一个网络时第一个包仍是直连，
/// 不先白等一轮中继（ADR-0004 的实测：只给中继 6031 毫秒，给直连地址 151 毫秒）。
const MAX_USED_ADDRS: usize = 8;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct Ticket {
    server: Server,
    base_domain: String,
    /// 配对时填的设备名，Matrix 设备显示名和 pusher 的设备名都用它。`ConnectHome` 给的连接信息里没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_name: Option<String>,
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
    /// 配对码和核心报告的候选。
    #[serde(default)]
    direct_addrs: Vec<SocketAddr>,
    /// 本机实际连通过的直连地址，最近的在前。核心的报告换掉候选时它不动。
    #[serde(default)]
    used_addrs: Vec<SocketAddr>,
}

impl Ticket {
    /// 连接信息里有没有中继表：endpoint 重建的证据只在有中继时凑得齐（ADR-0032 §4）。
    pub(crate) fn has_relays(&self) -> bool {
        !self.server.relays.is_empty()
    }

    /// 解析连接信息。至少要有一个中继地址或直连地址：只给 endpoint ID 时要靠地址发现，国内靠不住。
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let mut ticket: Self = serde_json::from_str(text).map_err(|e| {
            l10n!(
                CATALOG,
                language(),
                "home-ticket-invalid-json",
                error = e.to_string()
            )
        })?;
        if ticket.server.relays.is_empty()
            && ticket.server.direct_addrs.is_empty()
            && ticket.server.used_addrs.is_empty()
        {
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
        ticket.server.relays = ticket
            .server
            .relays
            .into_iter()
            .map(canonical_relay)
            .collect();
        ticket.server.used_addrs.truncate(MAX_USED_ADDRS);
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
            device_name: ticket
                .device_name
                .map(|name| name.trim().to_owned())
                .filter(|name| !name.is_empty()),
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

    /// 建连时同时试的全部地址：中继、核心的候选、本机连通过的直连地址。iroh 把第一个包同时发给所有
    /// 直连候选，多几条失效的只多几个包，不拖慢建连（`tests/tunnel/stale_addrs.rs`）。
    pub(crate) fn server_addr(&self) -> EndpointAddr {
        let mut addr = EndpointAddr::new(self.server.endpoint_id);
        for relay in &self.server.relays {
            addr = addr.with_relay_url(relay.url.clone());
        }
        for ip in self
            .server
            .direct_addrs
            .iter()
            .chain(&self.server.used_addrs)
        {
            addr = addr.with_ip_addr(*ip);
        }
        addr
    }

    /// 连接信息里的中继表，App 建 endpoint 用。
    pub(crate) fn relays(&self) -> &[Relay] {
        &self.server.relays
    }

    /// 记下一个本机实际连通过的直连地址，排在历史最前，最多留 [`MAX_USED_ADDRS`] 条；已经在最前时返回
    /// `None`。比核心的报告更可信（它不知道手机这一侧的地址转换），所以单独存、报告换候选时不动它。
    pub(crate) fn with_used(&self, addr: SocketAddr) -> Option<Self> {
        if self.server.used_addrs.first() == Some(&addr) {
            return None;
        }
        let mut updated = self.clone();
        updated.server.used_addrs.retain(|used| *used != addr);
        updated.server.used_addrs.insert(0, addr);
        updated.server.used_addrs.truncate(MAX_USED_ADDRS);
        Some(updated)
    }

    /// 记上核心报告的地址；和已有的一样时返回 `None`。中继表或直连地址报告为空时留着旧候选，仍把
    /// 本次实证直连路径排第一：中继可能只是暂时没连上，地址也可能只是这一次没测出来。令牌变了以后，
    /// 跑着的 endpoint 不会跟着换（iroh#4642），要等下一次重建 endpoint。
    ///
    /// 本次真实连通的直连路径优先于核心的候选；核心不知道手机侧的地址转换。报告非空时其它旧候选不合并，
    /// 避免失效地址积累并挤掉新候选（ADR-0032）；连通过的地址另记进历史，跨网络保留。
    pub(crate) fn with_reported(
        &self,
        relays: Vec<Relay>,
        direct_addrs: &[SocketAddr],
        connected: Option<SocketAddr>,
    ) -> Option<Self> {
        let mut updated = self.clone();
        let used = connected.and_then(|addr| updated.with_used(addr));
        let used_changed = used.is_some();
        if let Some(used) = used {
            updated = used;
        }
        if !relays.is_empty() {
            updated.server.relays = relays.into_iter().map(canonical_relay).collect();
        }
        if connected.is_some() || !direct_addrs.is_empty() {
            let candidates = if direct_addrs.is_empty() {
                self.direct_addrs()
            } else {
                direct_addrs
            };
            let mut addresses = Vec::with_capacity(MAX_DIRECT_ADDRS);
            for addr in connected.into_iter().chain(candidates.iter().copied()) {
                if !addresses.contains(&addr) {
                    addresses.push(addr);
                }
                if addresses.len() == MAX_DIRECT_ADDRS {
                    break;
                }
            }
            updated.server.direct_addrs = addresses;
        }
        let changed = used_changed
            || updated.server.relays != self.server.relays
            || updated.server.direct_addrs != self.server.direct_addrs;
        changed.then_some(updated)
    }

    /// 码里给的服务端直连地址。
    pub(crate) fn direct_addrs(&self) -> &[SocketAddr] {
        &self.server.direct_addrs
    }

    /// 本机实际连通过的直连地址，最近的在前。
    pub(crate) fn used_addrs(&self) -> &[SocketAddr] {
        &self.server.used_addrs
    }

    pub(crate) fn web_url(&self) -> Option<&str> {
        self.web_url.as_deref()
    }

    /// 家里服务器的基础域名（小写，不带末尾的点）。
    pub(crate) fn base_domain(&self) -> &str {
        &self.base_domain
    }

    /// 配对时填的设备名；去掉首尾空白后为空也算没有。
    pub(crate) fn device_name(&self) -> Option<&str> {
        self.device_name.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use iroh::SecretKey;
    use wonton_tunnel::relays::RelayToken;

    use super::*;

    #[test]
    fn reported_relays_replace_the_saved_table_and_an_empty_report_keeps_the_old_one() {
        let id = SecretKey::generate().public();
        let text = format!(
            r#"{{"server": {{"endpoint_id": "{id}", "relays": [{{"url": "https://a.example./", "tier": 0}}], "direct_addrs": ["192.0.2.2:18482"]}},
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
        assert!(ticket.with_reported(Vec::new(), &[], None).is_none());
        let updated = ticket
            .with_reported(
                vec![Relay {
                    url: "https://b.example./".parse().unwrap(),
                    token: Some(RelayToken::new("tok")),
                    quic_port: None,
                    tier: 0,
                }],
                &[],
                None,
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

    /// 连通过的直连地址进历史，最近的在前、去重、有上限；核心的报告换候选时历史不动，
    /// 下一次冷启动的第一个包照样发给它。
    #[test]
    fn used_addresses_are_kept_newest_first_across_reports() {
        let id = SecretKey::generate().public();
        let text = format!(
            r#"{{"server": {{"endpoint_id": "{id}", "direct_addrs": ["192.0.2.2:18482"]}},
                "base_domain": "wonton.localhost"}}"#
        );
        let mut ticket = Ticket::parse(&text).unwrap();
        let first: SocketAddr = "192.168.0.2:18482".parse().unwrap();
        let second: SocketAddr = "198.51.100.7:18482".parse().unwrap();
        ticket = ticket.with_used(first).unwrap();
        // 同一个地址再来一次只在最前时返回 None。
        assert!(ticket.with_used(first).is_none());
        ticket = ticket.with_used(second).unwrap();
        let tried: std::collections::BTreeSet<SocketAddr> =
            ticket.server_addr().ip_addrs().copied().collect();
        assert_eq!(
            tried,
            [
                "198.51.100.7:18482".parse().unwrap(),
                "192.168.0.2:18482".parse().unwrap(),
                "192.0.2.2:18482".parse().unwrap(),
            ]
            .into_iter()
            .collect()
        );
        // 报告只换候选，历史保留；连通过的地址再出现时挪到最前，不重复。
        let report: Vec<SocketAddr> = vec!["203.0.113.9:18482".parse().unwrap()];
        let updated = ticket
            .with_reported(Vec::new(), &report, Some(first))
            .unwrap();
        assert_eq!(
            updated.server.direct_addrs,
            [first, "203.0.113.9:18482".parse().unwrap()]
        );
        assert_eq!(updated.server.used_addrs, [first, second]);
        // 报告换到别的网络后，历史里的地址仍在建连地址里（`first` 在候选和历史里各一份，只发一次）。
        let tried: std::collections::BTreeSet<SocketAddr> =
            updated.server_addr().ip_addrs().copied().collect();
        assert_eq!(tried.len(), 3);
        assert!(tried.contains(&second));
        assert!(tried.contains(&"203.0.113.9:18482".parse().unwrap()));
        // 只把历史记满，超出上限时丢最旧的。
        let mut many = Ticket::parse(&text).unwrap();
        for port in 1..=(MAX_USED_ADDRS as u16 + 1) {
            many = many
                .with_used(format!("192.168.1.{port}:18482").parse().unwrap())
                .unwrap();
        }
        assert_eq!(many.server.used_addrs.len(), MAX_USED_ADDRS);
        assert_eq!(
            many.server.used_addrs[0],
            format!("192.168.1.{}:18482", MAX_USED_ADDRS + 1)
                .parse()
                .unwrap()
        );
        assert!(
            !many
                .server
                .used_addrs
                .contains(&"192.168.1.1:18482".parse().unwrap())
        );
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
