//! 代理插件的声明、地址和可用性（ADR-0039）。地址只认声明，容器名沿用实例的命名规则。

use bollard::models::HealthStatusEnum;
use wonton_protocol::{EgressFailure, EgressProblem, EgressProxyProvider};

use super::address::ProxyAddress;
use crate::plugin::entry::Entry;
use crate::plugin::languages::Languages;
use crate::plugin::manager::{InstallError, PluginManager, PluginStatus};
use crate::plugin::manifest::{Manifest, PluginId};

pub(in crate::plugin) fn provides_proxy(manifest: &Manifest) -> bool {
    manifest
        .provides
        .as_ref()
        .is_some_and(|p| p.egress_proxy.is_some())
}

pub(in crate::plugin) struct Provider {
    pub id: PluginId,
    pub address: ProxyAddress,
    pub network: String,
    status: PluginStatus,
}

impl PluginManager {
    pub(in crate::plugin) fn egress_provider(&self) -> Result<Provider, InstallError> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let (id, entry) = plugins
            .iter()
            .find(|(_, e)| e.provides_egress_proxy)
            .ok_or_else(|| InstallError::NotInstalled("没有装能做出网代理的插件".to_owned()))?;
        let spec = entry
            .listed
            .as_ref()
            .and_then(|l| l.manifest.provides.as_ref())
            .and_then(|p| p.egress_proxy.as_ref())
            .ok_or_else(|| InstallError::Storage(format!("代理插件 {id} 的声明读不出来")))?;
        let container = self.cfg.instance.container(&id.slug(), &spec.service);
        let address = ProxyAddress::parse(&format!("http://{container}:{}", spec.port))
            .map_err(InstallError::Storage)?;
        Ok(Provider {
            id: id.clone(),
            address,
            network: self.cfg.instance.network(&id.slug()),
            status: entry.status.clone(),
        })
    }

    pub(super) fn egress_provider_view(&self, langs: &Languages) -> Option<EgressProxyProvider> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .find(|(_, e)| e.provides_egress_proxy)
            .map(|(id, entry)| EgressProxyProvider {
                id: id.to_string(),
                title: langs.pick_owned(&entry.title),
            })
    }

    pub(in crate::plugin) fn check_proxy_provider(
        &self,
        manifest: &Manifest,
        plugins: &std::collections::BTreeMap<PluginId, Entry>,
    ) -> Result<(), InstallError> {
        if provides_proxy(manifest)
            && let Some((id, _)) = plugins
                .iter()
                .find(|(id, entry)| **id != manifest.id && entry.provides_egress_proxy)
        {
            return Err(InstallError::Conflict(format!(
                "已经装了代理插件 {id}：同一时间只能有一个"
            )));
        }
        Ok(())
    }

    pub(super) async fn proxy_provider_problem(
        &self,
    ) -> Result<Option<EgressProblem>, InstallError> {
        let provider = self.egress_provider()?;
        let state = self
            .engine
            .inspect_container(&provider.address.host)
            .await?;
        let state = state.and_then(|container| container.state);
        let ready = state.as_ref().is_some_and(|state| {
            state.running == Some(true)
                && state
                    .health
                    .as_ref()
                    .and_then(|h| h.status)
                    .is_none_or(|h| h == HealthStatusEnum::HEALTHY || h == HealthStatusEnum::EMPTY)
        });
        if provider.status == PluginStatus::Running && ready {
            return Ok(None);
        }
        Ok(Some(EgressProblem {
            reason: EgressFailure::ProxyUnreachable,
            message: format!(
                "代理插件没在运行：{}（插件状态：{:?}；容器状态：{:?}；健康检查：{:?}）",
                provider.id,
                provider.status,
                state.as_ref().and_then(|s| s.status),
                state
                    .as_ref()
                    .and_then(|s| s.health.as_ref())
                    .and_then(|h| h.status)
            ),
        }))
    }
}
