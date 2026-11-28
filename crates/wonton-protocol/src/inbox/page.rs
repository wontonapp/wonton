use serde::{Deserialize, Serialize};

use super::action::InboxAction;

/// `page` 的载荷：一个 MCP Apps 的 `ui://` 资源或附件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PagePayload {
    /// 发送方的 MCP 资源（`ui://…`）；和 `file_id` 二选一。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_uri: Option<String>,
    /// 文件服务里的 HTML 附件；和 `resource_uri` 二选一。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    /// 一句说明，≤120 字。
    #[serde(default)]
    pub caption: String,
    /// 页面可以联网的域名名单（`https://`、`wss://`，可带 `*.` 子域），最多 20 个。
    /// 空名单页面完全断网。
    #[serde(default)]
    pub network_domains: Vec<String>,
    #[serde(default)]
    pub actions: Vec<InboxAction>,
}
