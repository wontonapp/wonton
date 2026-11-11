//! 插件设置页（ADR-0077）的客户端方法：读、只校验的预览、保存。

use wonton_protocol::{PluginSettingsUpdate, PluginSettingsView};

use super::{ClientCore, CoreError};

impl ClientCore {
    /// 读插件设置页：字段、当前值、默认值、用到它的服务。装着的版本没有设置时报 `unsupported`。
    pub async fn plugin_settings(&self, id: String) -> Result<PluginSettingsView, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.plugin_settings(&id).await })
            .await
    }

    /// 只校验、不保存：App 可以在页面上实时标红。返回填上候选值的视图。
    pub async fn preview_plugin_settings(
        &self,
        id: String,
        update: PluginSettingsUpdate,
    ) -> Result<PluginSettingsView, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.preview_plugin_settings(&id, &update).await })
            .await
    }

    /// 保存插件设置：核心按 Schema 校验，只重建用到它的服务；返回保存后的视图。
    pub async fn set_plugin_settings(
        &self,
        id: String,
        update: PluginSettingsUpdate,
    ) -> Result<PluginSettingsView, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.set_plugin_settings(&id, &update).await })
            .await
    }
}
