//! 私人 Bot 的持久限制和数据身份（#658）：撤权或恢复半路失败时，先记下它的匿名卷、把它停住。
//!
//! 命名卷有自己的名字，换容器不丢；匿名卷跟着容器走，删容器时 Docker 连卷一起删。这里在删之前按
//! “容器 / 挂载目标”记下卷名（存在 Bot 记录里，核心重启后还在），重建时填回计划的挂载源，容器就接回
//! 原来那份数据，镜像自带的匿名卷也一样。待恢复的 Bot 停着不动：插件启动、核心重启、别处的重建都不把
//! 它起起来，免得撤权还没恢复时又去写；只有它的重试（见 [`super::recovery_replace`]）会清掉这个限制。

use std::collections::{BTreeMap, BTreeSet};

use bollard::models::{Mount, MountType};

use super::bots::PluginBots;
use super::manager::{InstallError, PluginManager};
use super::manifest::PluginId;
use super::names::LABEL_PLUGIN;
use super::render::Deployment;
use crate::l10n;

/// 容器名 → 挂载目标 → Docker 卷名。
pub(super) type Volumes = BTreeMap<String, BTreeMap<String, String>>;

impl PluginManager {
    /// 恢复还没做完的 Bot。记录读不出来时当成没有：插件照样能启动。
    pub(super) fn restricted_bots(&self, id: &PluginId) -> BTreeSet<String> {
        self.store
            .bots()
            .load(id)
            .map(|bots| {
                bots.bots
                    .into_iter()
                    .filter(|(_, entry)| entry.restricted.is_some())
                    .map(|(bot, _)| bot)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 撤权：记下这个 Bot 的匿名卷、按 `reason` 停住它，之后只能经重试恢复。停不下来时报错，
    /// 限制和卷已经记下了，重试照样接得上。
    pub async fn restrict_bot(
        &self,
        id: &PluginId,
        bot: &str,
        reason: String,
    ) -> Result<(), InstallError> {
        let _allocating = self.bot_allocation.lock().await;
        if !self.store.bots().load(id)?.bots.contains_key(bot) {
            return Err(InstallError::NotFound(format!("插件 {id} 没有 Bot {bot}")));
        }
        let containers = self.bot_containers(id, bot).await?;
        self.remember_bot_volumes(id, bot, &containers).await?;
        self.mark_restricted(id, bot, reason).await?;
        self.stop_bot_containers(id, bot).await?;
        tracing::info!(plugin = %id, bot, "Bot 已停住，等恢复");
        Ok(())
    }

    /// 停这个 Bot 的容器（保留容器和卷）。
    pub(super) async fn stop_bot_containers(
        &self,
        id: &PluginId,
        bot: &str,
    ) -> Result<(), InstallError> {
        for name in self.bot_containers(id, bot).await? {
            self.engine.stop_container(&name).await?;
        }
        Ok(())
    }

    /// 把容器现有的匿名卷并进记录。只认没有本插件标签的卷：命名卷（核心建的）自带标签，不记。
    pub(super) async fn remember_bot_volumes(
        &self,
        id: &PluginId,
        bot: &str,
        containers: &[String],
    ) -> Result<(), InstallError> {
        let found = self.volumes_of(containers).await?;
        if found.is_empty() {
            return Ok(());
        }
        self.write_bot(id, bot, |entry| {
            for (container, targets) in &found {
                entry
                    .volumes
                    .entry(container.clone())
                    .or_default()
                    .extend(targets.clone());
            }
        })
    }

    /// 标成“还没恢复”，换成这次失败的原因（最新的失败就是现在要看的）。
    pub(super) async fn mark_restricted(
        &self,
        id: &PluginId,
        bot: &str,
        reason: String,
    ) -> Result<(), InstallError> {
        self.write_bot(id, bot, |entry| entry.restricted = Some(reason))
    }

    /// 部署半路先把限制放上（已经有原因就留着）：进程这时死掉，核心重启也不会把它起起来。
    pub(super) async fn mark_pending(&self, id: &PluginId, bot: &str) -> Result<(), InstallError> {
        let reason = l10n::text(
            &self.owner_languages(),
            "plugin-settings-bot-recovery-pending",
            &[],
        );
        self.write_bot(id, bot, |entry| {
            entry.restricted.get_or_insert(reason);
        })
    }

    /// 恢复成功：按这个 Bot 现在的容器换一份卷记录，清掉限制。
    pub(super) async fn recovered_bot(
        &self,
        id: &PluginId,
        bot: &str,
        containers: &[String],
    ) -> Result<(), InstallError> {
        let volumes = self.volumes_of(containers).await?;
        self.write_bot(id, bot, |entry| {
            entry.volumes = volumes;
            entry.restricted = None;
        })
    }

    /// 按记录把旧卷填回计划：同一个容器、同一个挂载目标接回原来那份。镜像自带的匿名卷不在计划里，
    /// 补一条只挂卷的挂载；计划里已经换成命名卷的目标不动。
    pub(super) async fn attach_bot_volumes(
        &self,
        id: &PluginId,
        bot: &str,
        plan: &mut Deployment,
    ) -> Result<(), InstallError> {
        let recorded = match self.store.bots().load(id)?.bots.get(bot) {
            Some(entry) if !entry.volumes.is_empty() => entry.volumes.clone(),
            _ => return Ok(()),
        };
        for container in &mut plan.containers {
            let Some(targets) = recorded.get(&container.name) else {
                continue;
            };
            let image_volumes = self.image_volumes(&container.image).await?;
            let Some(host) = container.body.host_config.as_mut() else {
                continue;
            };
            let mounts = host.mounts.get_or_insert_with(Vec::new);
            for (target, volume) in targets {
                match mounts
                    .iter_mut()
                    .find(|m| m.target.as_deref() == Some(target))
                {
                    Some(mount) if mount.source.is_none() => {
                        mount.source = Some(volume.clone());
                    }
                    Some(_) => {}
                    // 计划里没有了、镜像也不声明：配置真的删掉了这个挂载，不再接回去。
                    None if image_volumes.contains(target) => mounts.push(Mount {
                        target: Some(target.clone()),
                        source: Some(volume.clone()),
                        typ: Some(MountType::VOLUME),
                        ..Default::default()
                    }),
                    None => {}
                }
            }
        }
        Ok(())
    }

    /// 镜像声明的匿名卷目标（`VOLUME`）。
    async fn image_volumes(&self, image: &str) -> Result<BTreeSet<String>, InstallError> {
        Ok(self
            .engine
            .image_volumes(image)
            .await?
            .into_iter()
            .collect())
    }

    /// 容器现存的匿名卷。容器可能已经在失败回滚里没了，逐个看一眼。
    async fn volumes_of(&self, containers: &[String]) -> Result<Volumes, InstallError> {
        let mut volumes = Volumes::new();
        for name in containers {
            let Some(info) = self.engine.inspect_container(name).await? else {
                continue;
            };
            let mut targets = BTreeMap::new();
            for mount in info.mounts.unwrap_or_default() {
                if mount.typ.as_deref() != Some("volume") {
                    continue;
                }
                let (Some(volume), Some(target)) = (mount.name, mount.destination) else {
                    continue;
                };
                let ours = self
                    .engine
                    .volume_labels(&volume)
                    .await?
                    .is_some_and(|labels| labels.contains_key(LABEL_PLUGIN));
                if !ours {
                    targets.insert(target, volume);
                }
            }
            if !targets.is_empty() {
                volumes.insert(name.clone(), targets);
            }
        }
        Ok(volumes)
    }

    fn write_bot(
        &self,
        id: &PluginId,
        bot: &str,
        change: impl FnOnce(&mut super::bots::BotEntry),
    ) -> Result<(), InstallError> {
        let mut bots: PluginBots = self.store.bots().load(id)?;
        let Some(entry) = bots.bots.get_mut(bot) else {
            return Err(InstallError::NotFound(format!("插件 {id} 没有 Bot {bot}")));
        };
        change(entry);
        self.store.bots().save(id, &bots)?;
        self.changed();
        Ok(())
    }

    /// 待恢复的 Bot 一行原因，放进插件的异常列表（容器名由调用方照常列）。
    pub(super) fn restricted_notes(&self, id: &PluginId) -> Vec<String> {
        let bots = match self.store.bots().load(id) {
            Ok(bots) => bots,
            Err(_) => return Vec::new(),
        };
        let langs = self.owner_languages();
        bots.bots
            .into_iter()
            .filter_map(|(bot, entry)| {
                entry.restricted.map(|reason| {
                    l10n::text(
                        &langs,
                        "plugin-install-resident-restricted",
                        &[("bot", bot.into()), ("reason", reason.into())],
                    )
                })
            })
            .collect()
    }
}
