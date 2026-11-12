//! 插件设置页（ADR-0077）的请求：读、只校验的预览、保存。

use std::sync::Arc;

use wonton_protocol::{PluginSettingsUpdate, PluginSettingsView};

use super::api::{INSTALL_TIMEOUT, READ_TIMEOUT, to_core_error};
use super::{CoreError, PluginsLink};

impl PluginsLink {
    /// 读插件设置页（ADR-0077）。
    pub(crate) async fn plugin_settings(&self, id: &str) -> Result<PluginSettingsView, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.get(&["plugins", id, "settings"], &[], READ_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    /// 只校验、不保存：返回填上候选值的视图。
    pub(crate) async fn preview_plugin_settings(
        &self,
        id: &str,
        update: &PluginSettingsUpdate,
    ) -> Result<PluginSettingsView, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.post(
            &["plugins", id, "settings", "preview"],
            update,
            READ_TIMEOUT,
        )
        .await
        .map_err(to_core_error)
    }

    /// 保存插件设置：核心按 Schema 校验，然后只重建用到它的服务；返回保存后的视图。
    pub(crate) async fn set_plugin_settings(
        self: &Arc<Self>,
        id: &str,
        update: &PluginSettingsUpdate,
    ) -> Result<PluginSettingsView, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let result = api
            .put(&["plugins", id, "settings"], update, INSTALL_TIMEOUT)
            .await
            .map_err(to_core_error);
        self.refresh_installed();
        result
    }
}
