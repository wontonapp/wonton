//! 出网代理的管理接口（ADR-0023 的“接口：App 和核心”）。

use std::collections::BTreeSet;
use std::sync::Arc;

use ipnet::IpNet;
use wonton_protocol::{EgressProxySettings, EgressProxySource, EgressProxyUpdate};

use super::address::ProxyAddress;
use super::lists::overlaps;
use super::probe::{blocks, fresh};
use super::settings::{EgressConfig, StoredEgress};
use crate::plugin::core_address::layout_of;
use crate::plugin::languages::Languages;
use crate::plugin::manager::{InstallError, PluginManager, detached};

impl PluginManager {
    /// 先返回最近一次的结果，探测时间太久就在后台再测：探测要等 5 秒以上，不能拖慢设置页。
    pub fn egress_settings(
        self: &Arc<Self>,
        langs: &Languages,
    ) -> Result<EgressProxySettings, InstallError> {
        let config = self.egress_config()?;
        let stale = !self.egress.plugin_path().is_some_and(|p| fresh(&p));
        if config.url.is_some() && stale && self.egress.begin_probe() {
            let this = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(e) = this.probe_egress().await {
                    tracing::warn!(error = %e, "探测代理失败");
                }
                this.egress.end_probe();
            });
        }
        self.egress_view(&config, langs)
    }

    fn egress_view(
        &self,
        config: &EgressConfig,
        langs: &Languages,
    ) -> Result<EgressProxySettings, InstallError> {
        let text = |cidrs: &[IpNet]| cidrs.iter().map(ToString::to_string).collect();
        let configured = config.url.is_some();
        Ok(EgressProxySettings {
            url: (config.source != EgressProxySource::Plugin)
                .then(|| config.url.as_ref().map(ProxyAddress::masked))
                .flatten(),
            source: config.source,
            provider: self.egress_provider_view(langs),
            plugin_path: self.egress.plugin_path(),
            core_path: self.egress.core_path(),
            host_cidrs: text(&self.egress.host_cidrs().unwrap_or_default()),
            direct_cidrs: text(&config.direct_cidrs),
            notices: self.egress.notices(),
            restarts: self.egress_restarts(!configured, langs)?,
        })
    }

    /// 改设置。请求断开了也照样做完（见 [`detached`]）。
    pub async fn set_egress_proxy(
        self: &Arc<Self>,
        update: EgressProxyUpdate,
        langs: Languages,
    ) -> Result<EgressProxySettings, InstallError> {
        let this = Arc::clone(self);
        detached(async move { this.change_egress(update, &langs).await }).await
    }

    async fn change_egress(
        &self,
        update: EgressProxyUpdate,
        langs: &Languages,
    ) -> Result<EgressProxySettings, InstallError> {
        let _changing = self.egress.changing.lock().await;
        let current = self.egress_config()?;
        let target = if update.use_plugin {
            if update
                .url
                .as_ref()
                .is_some_and(|url| !url.trim().is_empty())
            {
                return Err(InstallError::Invalid(
                    "用代理插件时不能同时填代理地址".to_owned(),
                ));
            }
            if current.source == EgressProxySource::Env {
                return Err(env_locked());
            }
            Some(self.egress_provider()?.address)
        } else {
            target_address(&current, update.url.as_deref())?
        };
        let direct_cidrs = self.checked_cidrs(&update.direct_cidrs).await?;
        self.check_restarts(target.is_some(), &update.restarts)?;
        if let Some(proxy) = target.as_ref().filter(|t| {
            Some(*t) != current.url.as_ref()
                || (update.use_plugin && current.source != EgressProxySource::Plugin)
        }) {
            self.ensure_relay_network().await?;
            let problem = if update.use_plugin {
                let provider = self.egress_provider()?;
                match self.proxy_provider_problem().await? {
                    Some(problem) => Some(problem),
                    None => self.diagnose_at(proxy, &provider.network).await?,
                }
            } else {
                self.diagnose_at(proxy, &self.cfg.instance.egress()).await?
            };
            if let Some(problem) = problem.filter(blocks) {
                return Err(InstallError::EgressProxyUnavailable {
                    reason: problem.reason,
                    message: problem.message,
                });
            }
        }
        // 地址由环境变量指定时，存下的那份不动：环境变量取消后接着用。没有时不能写“清空了”：那是主人没做过的选择。
        let mut stored = if current.source == EgressProxySource::Env {
            self.store
                .egress_settings()?
                .unwrap_or_else(|| StoredEgress {
                    url: self.egress.detected_text(),
                    ..StoredEgress::default()
                })
        } else {
            StoredEgress {
                url: target
                    .as_ref()
                    .filter(|_| !update.use_plugin)
                    .map(|proxy| proxy.text().to_owned()),
                use_plugin: update.use_plugin,
                ..StoredEgress::default()
            }
        };
        stored.direct_cidrs = direct_cidrs;
        self.store.save_egress_settings(&stored)?;
        self.follow_core_proxy();
        self.egress.record_probes(None, None);
        self.apply_egress().await?;
        self.probe_egress().await?;
        self.egress_view(&self.egress_config()?, langs)
    }

    fn check_restarts(&self, configured: bool, confirmed: &[String]) -> Result<(), InstallError> {
        let expected: BTreeSet<String> = self
            .egress_pending(configured)?
            .iter()
            .map(ToString::to_string)
            .collect();
        let confirmed: BTreeSet<String> = confirmed.iter().cloned().collect();
        if expected != confirmed {
            return Err(InstallError::Changed(
                "要重启的插件和确认时不一样了：重新确认".to_owned(),
            ));
        }
        Ok(())
    }

    /// 主人多放行的网段：和 Docker 网络重叠就放不了行，直接告诉主人，不存下来等它失效。
    async fn checked_cidrs(&self, cidrs: &[String]) -> Result<Vec<IpNet>, InstallError> {
        let mut parsed = Vec::with_capacity(cidrs.len());
        for text in cidrs {
            let cidr: IpNet = text
                .parse()
                .map_err(|e| InstallError::Invalid(format!("网段 {text} 不合法：{e}")))?;
            parsed.push(cidr);
        }
        let mut networks = self.egress_networks();
        networks.insert(self.cfg.instance.egress());
        for network in networks {
            let found = self.engine.inspect_network(&network).await?;
            let Some(layout) = found.as_ref().and_then(layout_of) else {
                continue;
            };
            let subnet = IpNet::V4(layout.subnet);
            if let Some(cidr) = parsed.iter().find(|cidr| overlaps(cidr, &subnet)) {
                return Err(InstallError::Invalid(format!(
                    "网段 {cidr} 和 Docker 网络 {network}（{subnet}）重叠：放行的话出口容器会直接拨到别的插件"
                )));
            }
        }
        Ok(parsed)
    }
}

fn env_locked() -> InstallError {
    InstallError::Conflict(
        "代理地址由 WONTON_EGRESS_PROXY 指定，不能在这里改：改安装目录的设置后再执行一次安装脚本"
            .to_owned(),
    )
}

/// 主人要的地址。和 `GET` 给的打码地址一样时是不改：壳拿不到密码，改网段时带回来的就是它。
fn target_address(
    current: &EgressConfig,
    url: Option<&str>,
) -> Result<Option<ProxyAddress>, InstallError> {
    let url = url.map(str::trim).filter(|url| !url.is_empty());
    let unchanged = current.source != EgressProxySource::Plugin
        && url.map(str::to_owned) == current.url.as_ref().map(ProxyAddress::masked);
    if unchanged {
        return Ok(current.url.clone());
    }
    if current.source == EgressProxySource::Env {
        return Err(env_locked());
    }
    url.map(ProxyAddress::parse)
        .transpose()
        .map_err(InstallError::Invalid)
}
