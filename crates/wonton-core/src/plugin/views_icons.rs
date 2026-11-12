//! 管理接口的图标 PNG：已安装插件和最近预览过的插件包的图标，按变体渲染（ADR-0017 的“接口：App 和核心”）。

use super::manager::PluginManager;
use super::{gallery::Variant, image_worker::ImageError};

impl PluginManager {
    /// 图标地址对应的 PNG：只有已安装的插件和最近预览过的插件包的图标。不认识的地址为 `Ok(None)`。
    pub async fn icon_png(
        &self,
        sha256: &str,
        variant: Variant,
    ) -> Result<Option<Vec<u8>>, ImageError> {
        let tab = matches!(variant, Variant::TabSmall | Variant::TabLarge);
        let installed = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins
                .values()
                .filter_map(|entry| entry.listed.as_ref())
                .flat_map(|listed| {
                    listed
                        .tab_icons
                        .iter()
                        .flat_map(|(icon, selected)| {
                            [icon.as_ref(), selected.as_ref()]
                                .into_iter()
                                .flatten()
                                .map(|icon| (icon.clone(), true))
                        })
                        .chain(listed.icon.as_ref().map(|icon| (icon.clone(), false)))
                })
                .find(|(icon, is_tab)| icon.sha256 == sha256 && *is_tab == tab)
        };
        let Some((source, is_tab)) = installed.or_else(|| {
            (!tab)
                .then(|| self.gallery.preview(sha256))
                .flatten()
                .map(|source| (source, false))
        }) else {
            return Ok(None);
        };
        let icon = if is_tab {
            self.gallery.render_tab(&source).await?
        } else {
            self.gallery.render(&source).await?
        };
        Ok(Some(match variant {
            Variant::Small | Variant::TabSmall => icon.small.clone(),
            Variant::Large | Variant::TabLarge => icon.large.clone(),
        }))
    }
}
