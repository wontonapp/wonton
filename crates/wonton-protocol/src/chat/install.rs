use serde::{Deserialize, Serialize};

use crate::{InstallProgress, PluginFailure, PluginIcon, PluginOperation};

/// 聊天列表最上面的一行：有 Bot 的插件在装、在升级、装失败了。
/// 装好、Bot 的私聊出现以后它由私聊那一行代替，不并排显示。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotInstallRow {
    pub plugin_id: String,
    /// 插件名。
    pub title: String,
    /// 插件图标；没有时为空，壳画占位图。
    pub icon: Option<PluginIcon>,
    /// 新装还是升级。卸载不会出现在这里，卸载时私聊还在。
    pub operation: PluginOperation,
    /// 重试时重新预览的来源，如 `github:owner/repo@v1.2.0`、`upload:sha256:…`：和
    /// [`InstallPreview::source`](crate::InstallPreview::source) 一样交给预览接口，主人重新确认后再发
    /// [`PluginAction::InstallPlugin`](crate::PluginAction::InstallPlugin)。不知道来源时为空，壳不给重试，引导回插件页。
    pub source: Option<String>,
    pub state: BotInstallState,
}

/// 安装行的情况。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BotInstallState {
    Running {
        progress: InstallProgress,
    },
    /// 失败，插件已经回滚。`reason` 决定壳显示的类别和下一步，`message` 是默认收起的详情。
    Failed {
        reason: PluginFailure,
        /// 家里的服务器写的说明，按 App 的语言。
        message: String,
    },
    /// 说不清做到哪一步（连接断了、家里的服务器一直没回应）：可能还在装，看插件列表。
    Uncertain {
        /// 家里的服务器或客户端核心写的说明，按 App 的语言。
        message: String,
    },
}
