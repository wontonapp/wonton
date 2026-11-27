//! Bot 自己的稳定卷（compose 里的匿名卷、镜像的 `VOLUME`，名字见 [`super::names::Instance::bot_volume`]）。
//! Docker 每建一次容器给一个新的匿名卷，删容器时连带删掉，换容器（改权限、收紧私人空间、重建、核心重启后续建）
//! 就丢数据；按名字挂的卷不随容器删，也不会和别的 Bot 混用。只有删 Bot 且不留数据、或卸载插件删数据时才删。

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use bollard::models::{ContainerCreateBody, Mount, MountType};

use super::leftovers::name_taken;
use super::lifecycle::Undo;
use super::manager::{InstallError, PluginManager};
use super::manifest::PluginId;
use super::names::LABEL_PLUGIN;
use super::render::{ContainerPlan, Deployment, LABEL_BOT};

impl PluginManager {
    /// 建 Bot 容器用的配置：镜像自带 `VOLUME`、计划里又没挂东西的挂载点也挂这个 Bot 的稳定卷，用到的稳定卷先按
    /// 标签建好、已有的核对归属。全插件一份的服务原样返回。新建的卷记进 `undo`，和插件的命名卷一样随回滚删。
    pub(super) async fn with_bot_volumes(
        &self,
        container: &ContainerPlan,
        plan: &Deployment,
        undo: &mut Undo,
    ) -> Result<ContainerCreateBody, InstallError> {
        let mut body = container.body.clone();
        let Some(bot) = container.bot.as_deref() else {
            return Ok(body);
        };
        let plugin = plan
            .labels
            .get(LABEL_PLUGIN)
            .and_then(|id| PluginId::parse(id).ok())
            .ok_or_else(|| InstallError::Invalid("渲染计划没有插件归属".to_owned()))?;
        let slug = plugin.slug();
        let image = body
            .image
            .clone()
            .unwrap_or_else(|| container.image.clone());
        let declared = self.engine.image_volumes(&image).await?;
        let instance = &self.cfg.instance;
        let host = body.host_config.get_or_insert_with(Default::default);
        let mut covered: BTreeSet<PathBuf> = host
            .mounts
            .iter()
            .flatten()
            .filter_map(|mount| mount.target.as_deref())
            .chain(
                host.tmpfs
                    .iter()
                    .flatten()
                    .map(|(target, _)| target.as_str()),
            )
            .map(normalized)
            .collect();
        let mounts = host.mounts.get_or_insert_with(Vec::new);
        for target in declared {
            if covered.insert(normalized(&target)) {
                mounts.push(Mount {
                    source: Some(instance.bot_volume(&slug, &container.service, bot, &target)),
                    target: Some(target),
                    typ: Some(MountType::VOLUME),
                    read_only: Some(false),
                    ..Default::default()
                });
            }
        }
        let volumes: Vec<String> = mounts
            .iter()
            .filter(|mount| mount.typ == Some(MountType::VOLUME))
            .filter_map(|mount| {
                let (source, target) = (mount.source.as_deref()?, mount.target.as_deref()?);
                (source == instance.bot_volume(&slug, &container.service, bot, target))
                    .then(|| source.to_owned())
            })
            .collect();
        for volume in volumes {
            self.ensure_bot_volume(&volume, &plugin, bot, &plan.labels, undo)
                .await?;
        }
        Ok(body)
    }

    /// 不存在就带标签建（插件、实例、Bot），之后按标签清；同名卷不是这个 Bot 的就拒绝，不挂别人的数据。
    async fn ensure_bot_volume(
        &self,
        name: &str,
        plugin: &PluginId,
        bot: &str,
        labels: &HashMap<String, String>,
        undo: &mut Undo,
    ) -> Result<(), InstallError> {
        let Some(found) = self.engine.volume_labels(name).await? else {
            let mut labels = labels.clone();
            labels.insert(LABEL_BOT.to_owned(), bot.to_owned());
            self.engine.create_volume(name, &labels).await?;
            undo.volumes.push(name.to_owned());
            return Ok(());
        };
        if self.cfg.instance.owns(&found)
            && found.get(LABEL_PLUGIN).map(String::as_str) == Some(plugin.as_str())
            && found.get(LABEL_BOT).map(String::as_str) == Some(bot)
        {
            return Ok(());
        }
        let langs = self.owner_languages();
        Err(name_taken(
            &crate::l10n::text(&langs, "plugin-install-word-volume", &[]),
            name,
            &found,
            &langs,
        ))
    }

    /// 删掉这个 Bot 的稳定卷（按标签找，只认这套核心的）。调用方先删容器：还挂着的卷删不掉。
    pub(super) async fn remove_bot_volumes(
        &self,
        plugin: &PluginId,
        bot: &str,
    ) -> Result<(), InstallError> {
        let instance = &self.cfg.instance;
        let found = self
            .engine
            .labeled(LABEL_PLUGIN, plugin.as_str(), |labels| {
                instance.owns(labels) && labels.get(LABEL_BOT).map(String::as_str) == Some(bot)
            })
            .await?;
        for volume in &found.volumes {
            self.engine.remove_volume(volume).await?;
        }
        Ok(())
    }
}

fn normalized(target: &str) -> PathBuf {
    Path::new(target).components().collect()
}
