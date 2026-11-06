//! 插件的工具权限（ADR-0024），单独放一个文件，`client.rs` 不超过行数上限。

use wonton_protocol::{CoreError, ToolPolicyView};

use super::ClientCore;

#[uniffi::export]
impl ClientCore {
    /// 插件的工具权限（ADR-0024）：工具、始终允许、一律允许、生效中的放行时段。
    pub async fn tool_policy(&self, id: String) -> Result<ToolPolicyView, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.tool_policy(&id).await })
            .await
    }

    /// 这个 Bot（`caller`）调这个插件不再问，包括会改数据的工具；返回改完的设置。
    pub async fn set_tool_always(
        &self,
        id: String,
        caller: String,
        enabled: bool,
    ) -> Result<ToolPolicyView, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move {
                plugins
                    .set_tool_policy(&id, "always", &caller, enabled)
                    .await
            })
            .await
    }

    /// 这个插件自己的 Bot（`caller`）调所有插件不再问，以后装的也算；返回改完的设置。
    pub async fn set_tool_allow_all(
        &self,
        id: String,
        caller: String,
        enabled: bool,
    ) -> Result<ToolPolicyView, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move {
                plugins
                    .set_tool_policy(&id, "allow-all", &caller, enabled)
                    .await
            })
            .await
    }

    /// 主人把这个工具标成只读，所有 Bot 调它都不再问；返回改完的设置。
    pub async fn set_tool_read_only(
        &self,
        id: String,
        tool: String,
        enabled: bool,
    ) -> Result<ToolPolicyView, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move {
                plugins
                    .set_tool_policy(&id, "read-only", &tool, enabled)
                    .await
            })
            .await
    }

    /// 停止这张卡片（`card`）打开的放行时段：已经结束的也算成功；返回停止后的设置。
    pub async fn stop_tool_window(
        &self,
        id: String,
        card: String,
    ) -> Result<ToolPolicyView, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.stop_tool_window(&id, &card).await })
            .await
    }
}
