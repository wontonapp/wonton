//! 重建容器时把旧网段迁进地址段：主网络、局域网端口的网络、出口容器和它的网络（整套核心共用）。
//! 渲染之前预留新网段、按新网段算变量；渲染成功才删旧容器、换网络，渲染失败放掉预留什么都不动。

use std::net::Ipv4Addr;

use ipnet::Ipv4Net;

use super::core_address::{Reservation, last_usable};
use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::manifest::Manifest;
use super::upstreams::core_aliases;
use crate::l10n;

/// 渲染前准备好的迁移。各网络的新网段已经预留（同进程其他建网络跳过），按新主网络算出的
/// 变量也备好；主网络没迁时它们是 `None`/空，渲染照旧读现有网络。丢弃它就放掉预留：渲染
/// 失败时旧网络和旧容器一个没动，插件照原样跑。
pub(in crate::plugin) struct PendingMigration {
    pub(in crate::plugin) main: Option<Ipv4Net>,
    pub(in crate::plugin) lan: Option<Ipv4Net>,
    pub(in crate::plugin) egress: Option<Ipv4Net>,
    pub(in crate::plugin) core_address: Option<String>,
    pub(in crate::plugin) extra_hosts: Vec<String>,
}

impl PluginManager {
    /// 一个插件可能挂在核心自建的三种网络上，旧网段（旧版核心让 Docker 从默认地址池分的）在
    /// 重建容器时都要迁进地址段：主网络（每个插件一个）、局域网端口的网络（`--lan`，开了
    /// `network.lan` 才有）、出口容器和它的网络（`--egress`，整套核心共用一个）。不在段外的网络
    /// 不动。出口网络是共用的：迁它要重建出口容器，再把所有出网插件接回来。
    pub(in crate::plugin) async fn prepare_migration(
        &self,
        manifest: &Manifest,
    ) -> Result<Option<(PendingMigration, Reservation)>, InstallError> {
        let slug = manifest.id.slug();
        let main_outside = self
            .engine
            .network_outside_pool(&self.cfg.instance.network(&slug))
            .await?;
        let lan_outside = self
            .engine
            .network_outside_pool(&self.cfg.instance.lan_network(&slug))
            .await?;
        let egress_outside = self
            .engine
            .network_outside_pool(&self.cfg.instance.egress())
            .await?;
        let count =
            usize::from(main_outside) + usize::from(lan_outside) + usize::from(egress_outside);
        if count == 0 {
            return Ok(None);
        }
        let mut reservation = self
            .engine
            .reserve_subnets(count, &self.cfg.instance.network(&slug))
            .await?;
        let main = main_outside.then(|| reservation.next().expect("预留数和要迁的网络数一致"));
        let lan = lan_outside.then(|| reservation.next().expect("预留数和要迁的网络数一致"));
        let egress = egress_outside.then(|| reservation.next().expect("预留数和要迁的网络数一致"));
        let (core_address, extra_hosts) = match main {
            // 渲染要按新网络算变量：核心地址（sso:proxy 的信任地址）和宿主核心的网关 hosts。
            Some(subnet) => {
                let address = match &self.cfg.core_container {
                    Some(_) => last_usable(subnet)
                        .expect("/26 有核心的保留地址")
                        .to_string(),
                    None => Ipv4Addr::from(subnet.network().to_bits() + 1).to_string(),
                };
                let aliases = core_aliases(&self.cfg.public, manifest);
                let extra_hosts = if self.cfg.core_container.is_none() && !aliases.is_empty() {
                    aliases
                        .into_iter()
                        .map(|alias| {
                            format!("{alias}:{}", Ipv4Addr::from(subnet.network().to_bits() + 1))
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                (Some(address), extra_hosts)
            }
            None => (None, Vec::new()),
        };
        Ok(Some((
            PendingMigration {
                main,
                lan,
                egress,
                core_address,
                extra_hosts,
            },
            reservation,
        )))
    }

    /// 渲染成功后的破坏阶段：删旧容器、按预留网段重建网络、接回核心和出口容器。走到这里渲染
    /// 已经过了，旧网段换新不会再改变量。这时旧容器已经删了，任何一步失败插件都起不来，错误
    /// 里写清楚。来源登记跟着换：网关按来源地址认插件，段变了要重新记。
    pub(in crate::plugin) async fn apply_migration(
        &self,
        manifest: &Manifest,
        pending: &PendingMigration,
        reservation: Reservation,
        egress_plugin: bool,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        // 网络建好（或失败）后预留都放掉：Docker 的网络列表里已经看得见，held 里不留。
        let result = self
            .apply_migration_inner(manifest, pending, egress_plugin)
            .await;
        drop(reservation);
        result.map_err(|e| {
            e.with_note(
                langs,
                &l10n::text(langs, "plugin-settings-note-migration-containers-gone", &[]),
            )
        })
    }

    async fn apply_migration_inner(
        &self,
        manifest: &Manifest,
        pending: &PendingMigration,
        egress_plugin: bool,
    ) -> Result<(), InstallError> {
        let slug = manifest.id.slug();
        let network = self.cfg.instance.network(&slug);
        let found = self.own_resources(&manifest.id).await?;
        for container in &found.containers {
            self.engine.remove_container(container).await?;
        }
        // 出口网络先迁：迁它要重建共用的出口容器，主网络重建后还要把它接回来。
        if let Some(subnet) = pending.egress {
            let name = self.cfg.instance.egress();
            self.engine.remove_container(&name).await?;
            self.engine.remove_network(&name).await?;
            self.engine
                .create_relay_network_at(&name, &self.cfg.instance.component_labels(), subnet)
                .await?;
            let config = self.egress_config()?;
            if let Some(proxy) = &config.url {
                self.sync_relay(proxy, &config, None).await?;
            }
        }
        if let Some(subnet) = pending.main {
            if let Some(core) = &self.cfg.core_container {
                self.engine.disconnect_network(&network, core).await?;
            }
            self.detach_relay(&network).await?;
            self.detach_mailpit(&network).await?;
            self.engine.remove_network(&network).await?;
            self.engine
                .create_network_at(
                    &network,
                    &self.cfg.instance.labels(&manifest.id),
                    self.network_reach(manifest),
                    subnet,
                )
                .await?;
            if let Some(core) = &self.cfg.core_container {
                self.attach_core(&network, core, manifest).await?;
            }
            if egress_plugin {
                self.attach_relay(&network).await?;
            }
        }
        if let Some(subnet) = pending.lan {
            let lan_network = self.cfg.instance.lan_network(&slug);
            self.engine.remove_network(&lan_network).await?;
            self.engine
                .create_lan_network_at(
                    &lan_network,
                    &self.cfg.instance.labels(&manifest.id),
                    subnet,
                )
                .await?;
        }
        self.register_source(manifest, &network).await
    }
}
