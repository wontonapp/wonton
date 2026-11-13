//! 安装、升级、卸载的进展（`PluginsViewModel.tasks`）：核心在后台做完，App 只看进展和结果。

use serde::{Deserialize, Serialize};

use super::{AddedTools, InstallProgress, InstalledPlugin, PluginFailure, PluginIcon};

/// 一次安装、升级或卸载。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginTask {
    /// 插件 ID。
    pub id: String,
    pub title: String,
    pub icon: Option<PluginIcon>,
    pub operation: PluginOperation,
    pub state: PluginTaskState,
    /// 家里的服务器报到的那一步（拉镜像、准备、启动、等 Bot 上线）：做的时候跟着已安装列表走，
    /// 壳只显示阶段，不显示没有的百分比。没在做或核心没报时为空。
    pub install_progress: Option<InstallProgress>,
    /// 请求发出后连接断了，在等连接回来：`state` 还是 `Running`，结果不知道。等连接时核心不自动重试。
    pub waiting_for_connection: bool,
    /// 升级比旧版本多了哪些工具；不是升级、没做完或者等不到结果时为空。
    pub added_tools: Option<AddedTools>,
}

/// `POST /_wonton/api/plugins/{id}/upgrade` 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeResult {
    pub plugin: InstalledPlugin,
    pub added_tools: AddedTools,
}

/// 任务做的是什么。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PluginOperation {
    Install,
    Upgrade,
    Uninstall,
}

/// 任务的进展。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginTaskState {
    /// 家里的服务器在做；App 退到后台或者断开了，它也照样做完。
    Running,
    /// 做完了，已安装列表随之更新。
    Succeeded,
    Failed {
        reason: PluginFailure,
        message: String,
    },
    /// 等不到结果：家里的服务器可能已经做完了，要看已安装列表。
    Uncertain {
        message: String,
        next_step: PluginTaskNextStep,
    },
}

impl PluginTaskState {
    /// 还没做完（包括在等连接）：主人不能清掉这条任务，也不能对同一个插件再发一次。
    pub fn in_progress(&self) -> bool {
        matches!(self, Self::Running)
    }
}

/// 结果不确定以后，界面给主人的下一步。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PluginTaskNextStep {
    /// 到已安装列表里看插件在不在，不要直接再装一遍。
    CheckInstalled,
}
