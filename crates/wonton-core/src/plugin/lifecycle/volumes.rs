//! 插件声明的命名卷：新建、复用同名卷或拒绝安装；安装失败时按 [`Undo::volumes`] 删掉本次建的。

use std::collections::HashMap;

use super::super::languages::Languages;
use super::super::leftovers::name_taken;
use super::super::manager::{InstallError, PluginManager};
use super::super::manifest::{Manifest, PluginId};
use super::super::names::LABEL_PLUGIN;
use super::super::render::Deployment;
use super::Undo;
use crate::l10n;

/// 插件声明的命名卷怎么处理。
#[derive(Debug, PartialEq, Eq)]
pub(in crate::plugin) enum VolumeAction {
    Create,
    Reuse,
}

/// Docker 创建同名卷时直接返回已有的卷，所以先看它属于谁：不存在就新建（安装失败时回滚删除）；
/// 属于本插件（上次不删数据卸载后保留的）就复用，回滚时不删；属于其他插件或不是核心创建的，拒绝安装。
pub(in crate::plugin) fn volume_action(
    volume: &str,
    labels: Option<&HashMap<String, String>>,
    plugin: &PluginId,
    langs: &Languages,
) -> Result<VolumeAction, InstallError> {
    let Some(labels) = labels else {
        return Ok(VolumeAction::Create);
    };
    if labels.get(LABEL_PLUGIN).map(String::as_str) == Some(plugin.as_str()) {
        Ok(VolumeAction::Reuse)
    } else {
        Err(name_taken(
            &l10n::text(langs, "plugin-install-word-volume", &[]),
            volume,
            labels,
            langs,
        ))
    }
}

impl PluginManager {
    /// 按计划建卷；已存在的卷不动（见 [`volume_action`]）。
    pub(in crate::plugin) async fn create_volumes(
        &self,
        plan: &Deployment,
        manifest: &Manifest,
        undo: &mut Undo,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        for volume in &plan.volumes {
            let labels = self.engine.volume_labels(volume).await?;
            if volume_action(volume, labels.as_ref(), &manifest.id, langs)? == VolumeAction::Create
            {
                self.engine.create_volume(volume, &plan.labels).await?;
                undo.volumes.push(volume.clone());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned_by(owner: Option<&str>) -> HashMap<String, String> {
        owner
            .map(|o| HashMap::from([(LABEL_PLUGIN.to_owned(), o.to_owned())]))
            .unwrap_or_default()
    }

    #[test]
    fn named_volumes_are_created_reused_or_refused_by_owner() {
        let plugin = PluginId::parse("app.foo-bar").unwrap();
        let langs = Languages::from_header(Some("en"));
        let action = |labels: Option<HashMap<String, String>>| {
            volume_action("wonton-app-foo-bar--data", labels.as_ref(), &plugin, &langs)
        };
        assert_eq!(action(None).unwrap(), VolumeAction::Create);
        assert_eq!(
            action(Some(owned_by(Some("app.foo-bar")))).unwrap(),
            VolumeAction::Reuse
        );
        // app.foo.bar 和 app.foo-bar 的 slug 相同，它卸载时保留的 data 卷同名：不能挂上别的插件的数据。
        let other = action(Some(owned_by(Some("app.foo.bar")))).unwrap_err();
        assert_eq!(other.kind(), "conflict", "{other}");
        let unlabeled = action(Some(owned_by(None))).unwrap_err();
        assert_eq!(unlabeled.kind(), "conflict", "{unlabeled}");
    }
}
