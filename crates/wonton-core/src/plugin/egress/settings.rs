//! 这台主机用哪个代理（ADR-0023 的“代理地址从哪里来”）：优先级和 ADR-0019 的 `WONTON_LAN_ADDRESSES` 一致，
//! `WONTON_EGRESS_PROXY` > App 里的设置（填的或者清空的，都存下来）> 安装脚本检测到的 `WONTON_EGRESS_PROXY_DETECTED`。
//! 额外直连的网段只在 App 里设。

use std::sync::Mutex;

use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use wonton_protocol::EgressProxySource;

use super::address::ProxyAddress;

/// 主人在 App 里存下的设置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredEgress {
    /// 没有时是主人清空了。
    pub url: Option<String>,
    #[serde(default)]
    pub direct_cidrs: Vec<IpNet>,
}

/// 现在生效的设置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressConfig {
    pub url: Option<ProxyAddress>,
    pub source: EgressProxySource,
    pub direct_cidrs: Vec<IpNet>,
}

/// 核心里出网代理的状态。
#[derive(Default)]
pub struct Egress {
    /// `WONTON_EGRESS_PROXY`：设了就只用它，设置里不能改。
    env: Option<ProxyAddress>,
    /// `WONTON_EGRESS_PROXY_DETECTED`：只作初值。
    detected: Option<ProxyAddress>,
    /// 出口容器一次只改一处：同时安装的两个插件不能同时建它、改名单。
    pub(super) pass: tokio::sync::Mutex<()>,
    /// 宿主所在网段，局域网检测时更新；还没检测过时为空。
    host_cidrs: Mutex<Option<Vec<IpNet>>>,
}

impl Egress {
    pub fn new(env: Option<ProxyAddress>, detected: Option<ProxyAddress>) -> Self {
        Self {
            env,
            detected,
            ..Self::default()
        }
    }

    pub(super) fn host_cidrs(&self) -> Option<Vec<IpNet>> {
        self.host_cidrs.lock().expect("出网代理状态锁中毒").clone()
    }

    /// 记下这次检测到的网段，返回和上次比变了没有。
    pub(super) fn set_host_cidrs(&self, cidrs: Vec<IpNet>) -> bool {
        let mut current = self.host_cidrs.lock().expect("出网代理状态锁中毒");
        let changed = current.as_ref() != Some(&cidrs);
        *current = Some(cidrs);
        changed
    }

    pub fn resolve(&self, stored: Option<&StoredEgress>) -> Result<EgressConfig, String> {
        let direct_cidrs = stored.map(|s| s.direct_cidrs.clone()).unwrap_or_default();
        let (url, source) = match (&self.env, stored, &self.detected) {
            (Some(env), _, _) => (Some(env.clone()), EgressProxySource::Env),
            (None, Some(stored), _) => {
                let url = stored.url.as_deref().map(ProxyAddress::parse).transpose()?;
                let source = if url.is_some() {
                    EgressProxySource::Settings
                } else {
                    EgressProxySource::Unset
                };
                (url, source)
            }
            (None, None, Some(detected)) => (Some(detected.clone()), EgressProxySource::Detected),
            (None, None, None) => (None, EgressProxySource::Unset),
        };
        Ok(EgressConfig {
            url,
            source,
            direct_cidrs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(text: &str) -> ProxyAddress {
        ProxyAddress::parse(text).unwrap()
    }

    fn stored(url: Option<&str>) -> StoredEgress {
        StoredEgress {
            url: url.map(str::to_owned),
            direct_cidrs: vec!["100.64.0.0/10".parse().unwrap()],
        }
    }

    #[test]
    fn the_install_setting_wins_then_the_app_then_what_was_detected() {
        let env = Some(address("http://env:1"));
        let detected = Some(address("http://detected:1"));
        let both = Egress::new(env, detected.clone());
        let app = stored(Some("http://app:1"));
        let config = both.resolve(Some(&app)).unwrap();
        assert_eq!(config.url, Some(address("http://env:1")));
        assert_eq!(config.source, EgressProxySource::Env);
        assert_eq!(config.direct_cidrs, app.direct_cidrs);

        let detected_only = Egress::new(None, detected);
        let config = detected_only.resolve(Some(&app)).unwrap();
        assert_eq!(config.url, Some(address("http://app:1")));
        assert_eq!(config.source, EgressProxySource::Settings);

        let config = detected_only.resolve(None).unwrap();
        assert_eq!(config.url, Some(address("http://detected:1")));
        assert_eq!(config.source, EgressProxySource::Detected);
    }

    #[test]
    fn clearing_it_in_the_app_is_remembered_over_what_was_detected() {
        let egress = Egress::new(None, Some(address("http://detected:1")));
        let config = egress.resolve(Some(&stored(None))).unwrap();
        assert_eq!(config.url, None);
        assert_eq!(config.source, EgressProxySource::Unset);

        let config = Egress::default().resolve(None).unwrap();
        assert_eq!(config.url, None);
        assert_eq!(config.source, EgressProxySource::Unset);
    }

    #[test]
    fn a_stored_address_that_no_longer_parses_is_an_error() {
        let egress = Egress::default();
        assert!(egress.resolve(Some(&stored(Some("ftp://x")))).is_err());
    }
}
