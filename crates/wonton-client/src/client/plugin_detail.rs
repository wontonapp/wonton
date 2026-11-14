//! 商店和详情页的取法：插件还在不在由核心判断（ADR-0017 的客户端核心接口），壳只照结果做。

use wonton_protocol::{
    CoreError, PluginAudience, PluginDetail, PluginDetailOutcome, PluginDetailRoute,
};

use super::ClientCore;

#[uniffi::export]
impl ClientCore {
    /// 商店里一个插件的详情（来源最新的版本）。`source` 是 [`wonton_protocol::StoreItem::source`]。
    pub async fn store_plugin(&self, source: String) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.store_plugin(&source).await })
            .await
    }

    /// 一个已安装插件的详情（装着的版本）。
    pub async fn installed_plugin(&self, id: String) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.installed_plugin(&id).await })
            .await
    }

    /// 详情页要显示的东西：`Close` 表示插件已经卸掉，壳关掉这一页。
    pub async fn plugin_detail(
        &self,
        route: PluginDetailRoute,
    ) -> Result<PluginDetailOutcome, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope.run(async move { plugins.detail(&route).await }).await
    }

    /// 详情页上的任务做完了：该关就关，否则重取详情。
    pub async fn plugin_detail_after_task(
        &self,
        route: PluginDetailRoute,
    ) -> Result<PluginDetailOutcome, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.detail_after_task(&route).await })
            .await
    }

    pub async fn plugin_audience(
        &self,
        id: String,
    ) -> Result<wonton_protocol::PluginAudience, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.plugin_audience(&id).await })
            .await
    }

    /// 等待角色授权落盘，再取详情；壳不先乐观改掉服务器给的选择。
    pub async fn update_plugin_audience(
        &self,
        id: String,
        audience: PluginAudience,
    ) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move {
                plugins
                    .put_plugin_audience(&id, &audience)
                    .await
                    .map_err(crate::plugins::api::to_core_error)?;
                plugins.refresh_installed();
                plugins.installed_plugin(&id).await
            })
            .await
    }
}
