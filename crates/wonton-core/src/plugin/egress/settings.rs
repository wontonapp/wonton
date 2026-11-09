//! 这台主机用哪个代理（ADR-0023 的“代理地址从哪里来”）：优先级和 ADR-0019 的 `WONTON_LAN_ADDRESSES` 一致，
//! `WONTON_EGRESS_PROXY` > App 里的设置（填的或者清空的，都存下来）> 安装脚本检测到的 `WONTON_EGRESS_PROXY_DETECTED`。
//! 额外直连的网段只在 App 里设。

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use url::Url;
use wonton_protocol::{EgressProbe, EgressProxySource};

use super::address::ProxyAddress;
use super::probe::Prober;

/// 配了出网代理时，链接预览经代理问这个 DoH 服务器解析域名。用 IP 写，不用先解析它自己的域名。
pub const DEFAULT_DOH: &str = "https://1.1.1.1/dns-query";

/// 默认的 DoH 服务器地址，常量本来就合法。
pub fn default_doh() -> Url {
    Url::parse(DEFAULT_DOH).expect("默认的 DoH 地址不合法")
}

/// 主人给的 DoH 地址：http(s)、有主机、没有用户名密码、带路径。
pub fn parse_doh(text: &str) -> Result<Url, String> {
    let url = Url::parse(text).map_err(|e| format!("不是合法的地址：{e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("要用 http:// 或 https://".into());
    }
    if url.host().is_none() {
        return Err("缺少主机名".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("不能带用户名或密码".into());
    }
    if url.path().is_empty() || url.path() == "/" {
        return Err("要带查询路径，比如 /dns-query".into());
    }
    Ok(url)
}

/// 主人在 App 里存下的设置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredEgress {
    /// 没有时是主人清空了。
    pub url: Option<String>,
    /// 经代理解析域名用的 DoH 服务器；没有时用 [`DEFAULT_DOH`]。App 里还不能改，经环境变量或设置文件改。
    #[serde(default)]
    pub doh_url: Option<String>,
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
    /// 经代理解析域名用的 DoH 服务器。
    pub doh_url: Url,
}

/// 核心里出网代理的状态。
#[derive(Default)]
pub struct Egress {
    /// `WONTON_EGRESS_PROXY`：设了就只用它，设置里不能改。
    env: Option<ProxyAddress>,
    /// `WONTON_EGRESS_PROXY_DETECTED`：只作初值。
    detected: Option<ProxyAddress>,
    /// `WONTON_DOH_URL`：设了就只用它，设置文件里不能改。
    doh: Option<Url>,
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

    /// `WONTON_DOH_URL`。
    #[must_use]
    pub fn with_doh(mut self, doh: Option<Url>) -> Self {
        self.doh = doh;
        self
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
                let url = stored
                    .url
                    .as_deref()
                    .map(|text| {
                        // 存过的地址装过时校验过；这里按默认语言报错（ADR-0043 的回退）。
                        ProxyAddress::parse(text, &crate::plugin::languages::Languages::default())
                    })
                    .transpose()?;
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
            doh_url: self.doh_url(stored)?,
        })
    }

    /// DoH 服务器：环境变量 > 设置文件 > 默认。
    fn doh_url(&self, stored: Option<&StoredEgress>) -> Result<Url, String> {
        if let Some(doh) = &self.doh {
            return Ok(doh.clone());
        }
        match stored.and_then(|stored| stored.doh_url.as_deref()) {
            Some(text) => parse_doh(text),
            None => Ok(default_doh()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(text: &str) -> ProxyAddress {
        ProxyAddress::parse(text, &Default::default()).unwrap()
    }

    fn stored(url: Option<&str>) -> StoredEgress {
        StoredEgress {
            url: url.map(str::to_owned),
            doh_url: None,
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

    #[test]
    fn the_doh_server_is_the_install_setting_then_the_stored_one_then_the_default() {
        let stored = StoredEgress {
            doh_url: Some("https://dns.example/dns-query".into()),
            ..Default::default()
        };
        let env: Url = "http://env.example/dns-query".parse().unwrap();
        let config = Egress::default()
            .with_doh(Some(env.clone()))
            .resolve(Some(&stored))
            .unwrap();
        assert_eq!(config.doh_url, env);

        let config = Egress::default().resolve(Some(&stored)).unwrap();
        assert_eq!(config.doh_url.as_str(), "https://dns.example/dns-query");

        let config = Egress::default().resolve(None).unwrap();
        assert_eq!(config.doh_url.as_str(), DEFAULT_DOH);
    }

    #[test]
    fn a_bad_doh_server_is_an_error() {
        for text in [
            "ftp://dns.example/dns-query",
            "https://dns.example/",
            "https://user:pass@dns.example/dns-query",
            "https://",
        ] {
            assert!(parse_doh(text).is_err(), "{text}");
        }
        assert!(parse_doh("https://1.1.1.1/dns-query").is_ok());
    }
}
