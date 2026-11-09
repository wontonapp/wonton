//! 一键安装、一键升级（ADR-0017）的接口：可更新的应用和核心决定要确认的操作。

use serde::{Deserialize, Serialize};

use super::InstallPreview;
use super::PluginIcon;

/// 一个有新版本可升级的应用（ADR-0017）：核心查到新版本的，加上商店里版本号和装着的不一样的。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UpdatablePlugin {
    pub plugin_id: String,
    pub title: String,
    /// 升级到的版本。
    pub version: String,
    /// 没有时壳画占位。
    pub icon: Option<PluginIcon>,
}

/// 核心取好预览、要主人拿主意的一键安装或升级（[`crate::Action::InstallNow`] 一类）：预览已经取好，
/// 确认页不再重新取；主人在确认页上改的（可选权限、密钥、设备）跟着预览原样交回。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StoreConfirmation {
    /// 装新应用。
    Install { preview: InstallPreview },
    /// 升级装着的应用。
    Upgrade { preview: InstallPreview },
}
