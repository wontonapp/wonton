//! 插件的工具权限（ADR-0024）：只读和改主人的放宽，都由壳在详情页上直接调，结果是改完之后的样子。

use reqwest::Method;
use wonton_protocol::{CoreError, ToolPolicyView};

use super::{PluginsLink, READ_TIMEOUT, to_core_error};

impl PluginsLink {
    pub(crate) async fn tool_policy(&self, id: &str) -> Result<ToolPolicyView, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.get(&["plugins", id, "tool-policy"], &[], READ_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    /// `section` 是 `always`、`allow-all` 或者 `read-only`，`subject` 是 Bot 或者工具。
    pub(crate) async fn set_tool_policy(
        &self,
        id: &str,
        section: &str,
        subject: &str,
        enabled: bool,
    ) -> Result<ToolPolicyView, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let method = if enabled { Method::PUT } else { Method::DELETE };
        let segments = ["plugins", id, "tool-policy", section, subject];
        api.empty(method, &segments, READ_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    /// 只拦后面的调用，已经发给插件的不撤回。
    pub(crate) async fn stop_tool_window(
        &self,
        id: &str,
        card: &str,
    ) -> Result<ToolPolicyView, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.stop_window(card, READ_TIMEOUT)
            .await
            .map_err(to_core_error)?;
        self.tool_policy(id).await
    }
}
