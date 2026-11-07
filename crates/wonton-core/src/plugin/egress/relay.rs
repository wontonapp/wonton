//! 核心怎么管出口容器：配了代理就让它在，接好每个出网插件的网络，名单跟着网络和宿主的网段走；取消了就删容器和配置，
//! 不删它的网络：子网不变，主人按网段写的防火墙规则继续有效。

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

use ipnet::IpNet;

use super::EgressVars;
use super::address::ProxyAddress;
use super::gost::{ALIAS, IMAGE, Relay, RelayFiles, config_yaml, runner};
use super::lists::{self, Reach};
use super::provider::provides_proxy;
use super::settings::EgressConfig;
use crate::plugin::compose::PullPolicy;
use crate::plugin::core_address::{Endpoint, layout_of};
use crate::plugin::lan::Detected;
use crate::plugin::lan::announce::Announced;
use crate::plugin::leftovers::name_taken;
use crate::plugin::manager::{InstallError, PluginManager, PluginStatus};
use crate::plugin::manifest::{Manifest, Permission};

impl PluginManager {
    pub(in crate::plugin) fn egress_config(&self) -> Result<EgressConfig, InstallError> {
        let stored = self.store.egress_settings()?;
        let mut config = self
            .egress
            .resolve(stored.as_ref())
            .map_err(InstallError::Storage)?;
        if config.source == wonton_protocol::EgressProxySource::Plugin {
            config.url = Some(self.egress_provider(&self.owner_languages())?.address);
        }
        Ok(config)
    }

    /// 设置读不出来时不用代理：设置页和探测都会报这个错。
    pub(in crate::plugin) fn follow_core_proxy(&self) {
        match self.egress_config() {
            Ok(config) => self.core_proxy.set(config.url.as_ref()),
            Err(e) => {
                tracing::warn!(error = %e, "读不出代理的设置");
                self.core_proxy.set(None);
            }
        }
    }

    /// 声明了 `network.egress` 且配了代理时，注入给插件的内容。
    pub(in crate::plugin) fn egress_vars(
        &self,
        manifest: &Manifest,
    ) -> Result<Option<EgressVars>, InstallError> {
        if provides_proxy(manifest)
            || !manifest.permissions.contains(&Permission::NetworkEgress)
            || self.egress_config()?.url.is_none()
        {
            return Ok(None);
        }
        Ok(Some(EgressVars {
            auth_host: self.cfg.public.auth_host(),
        }))
    }

    fn relay_files(&self) -> RelayFiles {
        RelayFiles::new(self.store.relay_dir(), runner())
    }

    /// 出网插件的网络：卸载中的不算。
    pub(super) fn egress_networks(&self) -> BTreeSet<String> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .filter(|(_, entry)| entry.status != PluginStatus::Uninstalling)
            .filter(|(_, entry)| {
                entry.listed.as_ref().is_some_and(|listed| {
                    let permissions = &listed.manifest.permissions;
                    permissions.contains(&Permission::NetworkEgress)
                })
            })
            .map(|(id, _)| self.cfg.instance.network(&id.slug()))
            .collect()
    }

    /// 安装出网插件时：出口容器要在，接入插件的网络。
    pub(in crate::plugin) async fn attach_relay(&self, network: &str) -> Result<(), InstallError> {
        let config = self.egress_config()?;
        let Some(proxy) = &config.url else {
            return Ok(());
        };
        self.ensure_host_cidrs().await?;
        let _pass = self.egress.pass.lock().await;
        self.sync_relay(proxy, &config, Some(network)).await
    }

    /// 卸载或回滚时，删网络之前断开：Docker 不让删还有端点的网络。
    pub(in crate::plugin) async fn detach_relay(&self, network: &str) -> Result<(), InstallError> {
        let _pass = self.egress.pass.lock().await;
        let name = self.cfg.instance.egress();
        if self.engine.endpoint(&name, network).await?.is_none() {
            return Ok(());
        }
        self.engine.disconnect_network(network, &name).await?;
        let config = self.egress_config()?;
        if config.url.is_some() {
            self.write_relay_lists(&config).await?;
        }
        Ok(())
    }

    /// 核心启动时和设置变了时：按设置建好或者删掉出口容器。
    pub async fn reconcile_relay(&self) -> Result<(), InstallError> {
        let config = self.egress_config()?;
        self.ensure_host_cidrs().await?;
        let _pass = self.egress.pass.lock().await;
        match &config.url {
            Some(proxy) => self.sync_relay(proxy, &config, None).await,
            None => {
                let name = self.cfg.instance.egress();
                self.engine.remove_container(&name).await?;
                if let Some(core) = &self.cfg.core_container
                    && self.engine.endpoint(core, &name).await?.is_some()
                {
                    self.engine.disconnect_network(&name, core).await?;
                }
                self.relay_files()
                    .remove(&self.owner_languages())
                    .map_err(InstallError::Storage)
            }
        }
    }

    pub(in crate::plugin) async fn sync_relay(
        &self,
        proxy: &ProxyAddress,
        config: &EgressConfig,
        extra: Option<&str>,
    ) -> Result<(), InstallError> {
        let name = self.cfg.instance.egress();
        let labels = self.cfg.instance.component_labels();
        self.ensure_relay_network().await?;
        let found = self.engine.inspect_container(&name).await?;
        let created = found.as_ref().and_then(|c| c.config.as_ref());
        if found.is_some() {
            let labels = created.and_then(|c| c.labels.clone()).unwrap_or_default();
            if !self.cfg.instance.is_component(&labels) {
                let langs = self.owner_languages();
                let what = crate::l10n::text(&langs, "plugin-install-word-container", &[]);
                return Err(name_taken(&what, &name, &labels, &langs));
            }
        }
        let current = created
            .and_then(|c| c.image.as_deref())
            .is_some_and(|image| {
                crate::plugin::registry_mirror::reference::same_image(image, IMAGE)
            });
        let state = found.as_ref().and_then(|c| c.state.as_ref());
        let running = state.and_then(|s| s.running) == Some(true);
        // 镜像拉不到时什么都不动，正在跑的出口容器继续跑。
        let mirror = self.registry_mirror_address(&self.owner_languages())?;
        let image = self
            .engine
            .ensure_image(IMAGE, PullPolicy::Missing, mirror.as_deref())
            .await
            .map_err(|error| {
                let langs = self.owner_languages();
                self.registry_pull_error(error, mirror.as_deref(), &langs)
            })?;
        let files = self.relay_files();
        let langs = self.owner_languages();
        let yaml = config_yaml(proxy, &langs).map_err(InstallError::Storage)?;
        let same = files.has_config(&yaml);
        if !same {
            // 先删容器再写配置：中途失败时下次还会重建，不会留下配置新、容器旧的状态。
            self.engine.remove_container(&name).await?;
            files
                .write_config(&yaml, &langs)
                .map_err(InstallError::Storage)?;
        }
        self.write_relay_lists(config).await?;
        if !same || !current || !running {
            let relay = Relay {
                name: &name,
                labels: &labels,
                files: &files,
                image: &image,
            };
            self.engine.start_relay(&relay).await?;
            tracing::info!(proxy = %proxy.masked(), "出口容器已启动");
        }
        let networks = self.egress_networks();
        let own = std::iter::once(name.as_str());
        for network in networks.iter().map(String::as_str).chain(extra).chain(own) {
            self.connect_relay(&name, network).await?;
        }
        // 核心探测插件的路径：在出口网络上发起，这个网络上的来源不算任何插件。
        if let Some(core) = &self.cfg.core_container {
            self.connect_relay_network(core, &name).await?;
        }
        self.write_relay_lists(config).await
    }

    /// 安装器的代理预检也经核心分配网络，启动后沿用已建的出口网络。
    pub(in crate::plugin) async fn ensure_relay_network(&self) -> Result<(), InstallError> {
        let name = self.cfg.instance.egress();
        let labels = self.cfg.instance.component_labels();
        match self.engine.network_labels(&name).await? {
            None => self.engine.create_relay_network(&name, &labels).await?,
            Some(found) if self.cfg.instance.is_component(&found) => {}
            Some(found) => {
                let langs = self.owner_languages();
                let what = crate::l10n::text(&langs, "plugin-install-word-network", &[]);
                return Err(name_taken(&what, &name, &found, &langs));
            }
        }
        Ok(())
    }

    async fn connect_relay_network(&self, core: &str, network: &str) -> Result<(), InstallError> {
        if self.engine.endpoint(core, network).await?.is_none() {
            let endpoint = Endpoint {
                gw_priority: Some(-1),
                ..Endpoint::default()
            };
            self.engine
                .connect_network_at(network, core, endpoint)
                .await?;
        }
        Ok(())
    }

    /// 接入一个插件网络。网关优先级 `-1`：默认路由留在它自己的网络上（理由同插件规范里核心的 `gw_priority`，#90）。
    /// 插件的网络不在时跳过：那个插件的核对会报告原因。
    async fn connect_relay(&self, name: &str, network: &str) -> Result<(), InstallError> {
        if self.engine.inspect_network(network).await?.is_none()
            || self.engine.endpoint(name, network).await?.is_some()
        {
            return Ok(());
        }
        let endpoint = Endpoint {
            address: None,
            aliases: vec![ALIAS.to_owned()],
            gw_priority: Some(-1),
        };
        self.engine
            .connect_network_at(network, name, endpoint)
            .await?;
        Ok(())
    }

    /// 按 gost 现在接着的网络和宿主的网段写名单。网段和接入的网络重叠的不放行，写进日志。
    async fn write_relay_lists(&self, config: &EgressConfig) -> Result<(), InstallError> {
        let mut reach = Reach {
            host_cidrs: self.egress.host_cidrs().unwrap_or_default(),
            direct_cidrs: config.direct_cidrs.clone(),
            ..Reach::default()
        };
        let name = self.cfg.instance.egress();
        for network in self.relay_networks(&name).await? {
            self.add_network(&network, &mut reach).await?;
        }
        // docker0 的网关：宿主上的服务常写成 `http://172.17.0.1:11434`。守护进程可以不建它（`"bridge": "none"`）。
        if let Some(found) = self.engine.inspect_network("bridge").await? {
            let gateway = layout_of(&found).and_then(|l| l.gateway);
            reach
                .gateways
                .extend(gateway.and_then(|g| g.parse::<IpAddr>().ok()));
        }
        let lists = lists::build(&self.cfg.public.base_domain, &reach);
        self.egress.set_notices(
            lists
                .not_allowed
                .iter()
                .map(|cidr| {
                    crate::l10n::text(
                        &self.owner_languages(),
                        "plugin-settings-egress-relay-cidr-overlap",
                        &[("cidr", cidr.to_string().into())],
                    )
                })
                .collect(),
        );
        self.relay_files()
            .write_lists(&lists, &self.owner_languages())
            .map_err(InstallError::Storage)
    }

    async fn add_network(&self, network: &str, reach: &mut Reach) -> Result<(), InstallError> {
        let Some(found) = self.engine.inspect_network(network).await? else {
            return Ok(());
        };
        if let Some(layout) = layout_of(&found) {
            reach.attached.push(IpNet::V4(layout.subnet));
            reach
                .gateways
                .extend(layout.gateway.and_then(|g| g.parse::<IpAddr>().ok()));
        }
        Ok(())
    }

    /// gost 接着的网络，包括它自己的。容器还没有时算上自己的网络：名单要在容器启动之前写好。
    async fn relay_networks(&self, name: &str) -> Result<Vec<String>, InstallError> {
        let found = self.engine.inspect_container(name).await?;
        let attached: BTreeMap<String, _> = found
            .and_then(|c| c.network_settings)
            .and_then(|s| s.networks)
            .map(|networks| networks.into_iter().collect())
            .unwrap_or_default();
        let own = std::iter::once(name.to_owned());
        let names: BTreeSet<String> = attached.into_keys().chain(own).collect();
        Ok(names.into_iter().collect())
    }

    /// 宿主所在网段还没检测过时检测一次。放在 [`Egress::pass`] 之外：局域网检测后拿着它的锁来更新名单。
    ///
    /// [`Egress::pass`]: super::settings::Egress
    async fn ensure_host_cidrs(&self) -> Result<(), InstallError> {
        if self.egress.host_cidrs().is_none() {
            let langs = self.owner_languages();
            let detected = self.detect_lan(&langs).await?;
            if detected.failure.is_none() {
                self.egress
                    .set_host_cidrs(host_cidrs(&detected, self.lan.announce.as_deref()));
            }
        }
        Ok(())
    }

    /// 局域网检测之后：地址或前缀变了只改名单。
    pub(in crate::plugin) async fn follow_egress(&self, detected: &Detected) {
        if detected.failure.is_some() {
            return;
        }
        if !self
            .egress
            .set_host_cidrs(host_cidrs(detected, self.lan.announce.as_deref()))
        {
            return;
        }
        let _pass = self.egress.pass.lock().await;
        let result = match self.egress_config() {
            Ok(config) if config.url.is_some() => self.write_relay_lists(&config).await,
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, "更新出口容器的名单失败");
        }
    }

    /// 配了代理：要检测宿主所在网段。
    pub(in crate::plugin) fn egress_configured(&self) -> bool {
        self.egress_config().is_ok_and(|c| c.url.is_some())
    }
}

/// 选出的局域网地址所在的网段（前缀来自检测容器读的 netlink），以及启动器声明的宣告网段。主人覆盖或者选的地址也这样查，
/// 查不到时不放行。宣告地址不在虚拟机的网卡上，用它带来的前缀（ADR-0023）。
fn host_cidrs(detected: &Detected, announce: Option<&[Announced]>) -> Vec<IpNet> {
    let addresses = || {
        detected
            .host
            .candidates
            .iter()
            .flat_map(|candidate| &candidate.addresses)
    };
    let bound: Vec<&IpAddr> = detected
        .result
        .as_ref()
        .map(|plan| plan.bind.iter().map(|bound| &bound.ip).collect())
        .unwrap_or_default();
    let mut nets: Vec<IpNet> = bound
        .into_iter()
        .filter_map(|ip| addresses().find(|address| address.ip == *ip))
        .filter_map(|address| IpNet::new(address.ip, address.prefix).ok())
        .map(|net| net.trunc())
        .collect();
    nets.extend(announce.unwrap_or_default().iter().filter_map(|a| a.cidr()));
    nets.sort();
    nets.dedup();
    nets
}
