//! 这台主机用哪个代理（ADR-0023 的“代理地址从哪里来”）：优先级和 ADR-0019 的 `WONTON_LAN_ADDRESSES` 一致，
//! `WONTON_EGRESS_PROXY` > App 里的设置（填的或者清空的，都存下来）> 安装脚本检测到的 `WONTON_EGRESS_PROXY_DETECTED`。
//! 额外直连的网段只在 App 里设。

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use wonton_protocol::{EgressProbe, EgressProxySource};

use super::address::ProxyAddress;
use super::probe::Prober;

/// 主人在 App 里存下的设置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredEgress {
    /// 没有时是主人清空了。
    pub url: Option<String>,
    #[serde(default)]
    pub direct_cidrs: Vec<IpNet>,
    #[serde(default)]
    pub use_plugin: bool,
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
    /// 改设置一次做一个：要重启插件，可能很久。
    pub(super) applying: tokio::sync::Mutex<()>,
    /// 改设置时，核对要重启的插件和存下设置之间不能插进别的修改。
    pub(in crate::plugin) changing: tokio::sync::Mutex<()>,
    /// 查原因的容器名字是固定的，一次起一个。
    pub(super) diagnosing: tokio::sync::Mutex<()>,
    prober: Prober,
    probing: AtomicBool,
    memory: Mutex<Memory>,
}

#[derive(Default)]
struct Memory {
    /// 宿主所在网段，局域网检测时更新；还没检测过时为空。
    host_cidrs: Option<Vec<IpNet>>,
    plugin_path: Option<EgressProbe>,
    core_path: Option<EgressProbe>,
    /// 放不了行的网段。
    notices: Vec<String>,
}

impl Egress {
    pub fn new(env: Option<ProxyAddress>, detected: Option<ProxyAddress>) -> Self {
        Self {
            env,
            detected,
            ..Self::default()
        }
    }

    /// 测试用：探测的结果由测试给出。
    #[cfg(test)]
    #[must_use]
    pub fn with_prober(mut self, prober: Prober) -> Self {
        self.prober = prober;
        self
    }

    fn memory(&self) -> std::sync::MutexGuard<'_, Memory> {
        self.memory.lock().expect("出网代理状态锁中毒")
    }

    pub(super) fn host_cidrs(&self) -> Option<Vec<IpNet>> {
        self.memory().host_cidrs.clone()
    }

    pub(super) fn prober(&self) -> &Prober {
        &self.prober
    }

    pub(super) fn plugin_path(&self) -> Option<EgressProbe> {
        self.memory().plugin_path.clone()
    }

    pub(super) fn core_path(&self) -> Option<EgressProbe> {
        self.memory().core_path.clone()
    }

    pub(super) fn record_probes(&self, plugin: Option<EgressProbe>, core: Option<EgressProbe>) {
        let mut memory = self.memory();
        memory.plugin_path = plugin;
        memory.core_path = core;
    }

    pub(super) fn notices(&self) -> Vec<String> {
        self.memory().notices.clone()
    }

    pub(super) fn set_notices(&self, notices: Vec<String>) {
        self.memory().notices = notices;
    }

    /// 同一时间只在后台探测一次。
    pub(super) fn begin_probe(&self) -> bool {
        !self.probing.swap(true, Ordering::SeqCst)
    }

    pub(super) fn end_probe(&self) {
        self.probing.store(false, Ordering::SeqCst);
    }

    /// 安装脚本检测到的地址，原文。
    pub(super) fn detected_text(&self) -> Option<String> {
        let detected = self.detected.as_ref();
        detected.map(|address| address.text().to_owned())
    }

    /// 记下这次检测到的网段，返回和上次比变了没有。
    pub(super) fn set_host_cidrs(&self, cidrs: Vec<IpNet>) -> bool {
        let mut memory = self.memory();
        let changed = memory.host_cidrs.as_ref() != Some(&cidrs);
        memory.host_cidrs = Some(cidrs);
        changed
    }

    pub fn resolve(&self, stored: Option<&StoredEgress>) -> Result<EgressConfig, String> {
        let direct_cidrs = stored.map(|s| s.direct_cidrs.clone()).unwrap_or_default();
        let (url, source) = match (&self.env, stored, &self.detected) {
            (Some(env), _, _) => (Some(env.clone()), EgressProxySource::Env),
            (None, Some(stored), _) if stored.use_plugin => (None, EgressProxySource::Plugin),
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
            use_plugin: false,
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
