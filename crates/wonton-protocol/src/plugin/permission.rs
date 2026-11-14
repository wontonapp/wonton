//! 插件要的权限和可选设备（ADR-0036、ADR-0057）：确认页和详情页按项显示，风险决定提示和确认的步数。

use serde::{Deserialize, Serialize};

use super::PluginWriter;

/// 权限的风险级别。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PermissionRisk {
    High,
    VeryHigh,
    Highest,
    /// 比这个 App 新的核心给的级别，按最高风险处理。
    #[default]
    #[serde(other)]
    Other,
}

/// 要授予插件的一项权限。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PermissionLine {
    /// 清单里的写法，如 `storage.share.photos:read`。
    pub permission: String,
    /// 给主人看的说明，如“读共享文件夹 photos”。
    pub text: String,
    /// 风险级别；普通权限为空，不认识的级别落到 [`PermissionRisk::Other`]。
    #[serde(default)]
    pub risk: Option<PermissionRisk>,
    /// 这项权限的风险提示；没有额外提示时为空。
    #[serde(default)]
    pub notice: Option<String>,
    /// 核心支持这项权限；不支持时插件装不了。
    pub supported: bool,
    /// 升级时新申请的，标出来。
    pub added: bool,
    /// 可选、默认关（ADR-0057）：主人自己在预览里打开，装好后能在详情页开关。列表里排在必需权限之后。
    #[serde(default)]
    pub optional: bool,
    /// 可选权限现在开着。新装时是 `false`，升级时是装着的版本的值；必需权限固定 `false`。
    #[serde(default)]
    pub enabled: bool,
    /// 关着时少了什么（清单 `optional_permissions` 的 `off_loss`，核心按 App 的语言选好）；必需权限为空。
    #[serde(default)]
    pub off_loss: Option<String>,
    /// 这项可选权限是谁在什么时候打开的；没开着时为空。
    #[serde(default)]
    pub opened: Option<PermissionOpened>,
}

/// 一项可选权限被打开的时间和人（ADR-0057）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PermissionOpened {
    /// Unix 秒。
    pub since: u64,
    /// 主人打开时为空，Bot 打开时是它自己。
    #[serde(default)]
    pub bot: Option<PluginWriter>,
}

/// 开关一项可选权限的请求体（ADR-0057）：打开时核心重建拿到字段的服务，返回更新后的 [`PluginDetail`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PermissionUpdate {
    pub enabled: bool,
}

/// 预览中可供主人选择、交给插件服务的一个宿主设备。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginDevice {
    /// 核心用来识别设备的稳定 ID。
    pub id: String,
    /// 给主人的设备名称。
    pub name: String,
    /// 设备在容器里的目标路径。
    pub target: String,
    /// 这个设备要交给的服务。
    pub service: String,
    /// 是否可以不选择。
    pub optional: bool,
    /// 设备属于哪一类（ADR-0036）；不认识的类落到 [`PluginDeviceKind::Other`]，壳提示先更新 App。
    #[serde(default)]
    pub kind: PluginDeviceKind,
    /// 是装着 Wonton 系统或者核心数据的那块盘；只有硬盘会标。
    #[serde(default)]
    pub system: bool,
    /// 主人在预览里选中了它（ADR-0036）：安装请求按选中的 `id` 提交，和 `lan_webui` 的开关一样在预览里改。
    #[serde(default)]
    pub chosen: bool,
}

/// 已装插件获授权的宿主设备，设备拔除后仍保留在详情页。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginGrantedDevice {
    /// 宿主路径；串口、硬盘通常为 by-id，USB 整类为 `/dev/bus/usb`。
    pub path: String,
    /// 安装记录中的名称；USB 整类没有单设备名称，由壳按类别本地化。
    #[serde(default)]
    pub name: Option<String>,
    /// 不认识的类别仍列出，不作为可选择设备提交。
    #[serde(default)]
    pub kind: PluginDeviceKind,
    #[serde(default)]
    pub system: bool,
    /// 授权路径在核心所见的宿主设备树中是否存在，不代表插件服务运行正常。
    pub available: bool,
}

/// 宿主设备的类别。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PluginDeviceKind {
    Serial,
    /// 整类 USB 授权，仅在已授权设备详情中列出，不提供单设备选择。
    Usb,
    Disk,
    /// 比这个 App 新的核心给的类别。
    #[default]
    #[serde(other)]
    Other,
}
