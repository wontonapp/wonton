//! 插件之间按能力依赖的协议类型（ADR-0053）：商店详情和确认页的缺失能力、提供方设置、
//! 卸载预览和升级拦截的类别。轻提示在 [`super::hint`]。

use serde::{Deserialize, Serialize};

use super::InstallRequest;

/// 一项能力在索引里的提供方（ADR-0053）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CapabilityProvider {
    /// 插件 ID。
    pub id: String,
    /// 插件标题。
    pub title: String,
    /// 安装来源：选中后用现有预览接口取该提供方完整的 [`super::InstallPreview`]。
    pub source: String,
    /// 是不是高权限能力（ADR-0053）：核心定义的按能力表，第三方的按这个提供方算（它申请了
    /// ADR-0036 的任一级高权限，或者它是 Agent 插件）。确认页照它醒目标注。
    pub high_privilege: bool,
}

/// 清单缺的一项能力（ADR-0053）：`requires` 没有提供方装不上；`wants` 照装，少点能力。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MissingCapability {
    /// 能力名。
    pub capability: String,
    /// 是不是第三方定义的能力（`x-<作者>.<名字>`），商店和确认页照它标注。
    pub third_party: bool,
    /// 索引里能提供它的插件；只有 requires 会带。空且是 requires 时表示商店里没有提供方。
    pub providers: Vec<CapabilityProvider>,
}

/// 卸载被拦时错误响应的额外字段（ADR-0053）：`{"error": {"kind": "in_use", "message": …, "plugins": […]}}`。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InUseError {
    /// 依赖这个插件（`requires` 它提供的能力）的插件：App 写「X、Y 在用」。
    pub plugins: Vec<CapabilityProviderRef>,
}

/// 安装请求里要一起装的提供方（ADR-0053）：预览里缺的 `requires` 能力选定的提供方。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InstallTogether {
    /// 插件 ID。
    pub id: String,
    /// 提供方自己的完整安装决定。内层 `install_together`、`connect_plugins` 都必须为空，否则输入非法。
    #[serde(deserialize_with = "provider_request")]
    pub request: InstallRequest,
}

fn provider_request<'de, D>(deserializer: D) -> Result<InstallRequest, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let request = InstallRequest::deserialize(deserializer)?;
    if !request.install_together.is_empty() || !request.connect_plugins.is_empty() {
        return Err(serde::de::Error::custom(
            "provider install request must not include install_together or connect_plugins",
        ));
    }
    Ok(request)
}

/// 一个装着的提供方插件（ADR-0053）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CapabilityProviderRef {
    /// 插件 ID。
    pub id: String,
    /// 插件标题。
    pub title: String,
    /// 它提供的能力对使用方是不是高权限（ADR-0053）：换提供方、接上它都要主人确认。核心定义的按
    /// 能力表，第三方的按这个提供方算。
    pub high_privilege: bool,
}

/// 设置里一种能力的提供方（ADR-0053）：同一能力可以装几个提供方，全局默认选一个。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CapabilitySetting {
    /// 能力名。
    pub capability: String,
    /// 是不是第三方定义的能力。
    pub third_party: bool,
    /// 装着的提供方，按装的先后。
    pub providers: Vec<CapabilityProviderRef>,
    /// 全局默认提供方的插件 ID；默认的被卸掉时，核心换成装得最早的。
    pub default_provider: String,
}

/// 每种能力的全局默认提供方（ADR-0053）：设置接口的响应。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CapabilityProviderSettings {
    /// 有装着的提供方的能力，一项一个。
    pub capabilities: Vec<CapabilitySetting>,
}

/// 改一种能力的全局默认提供方（ADR-0053）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CapabilityDefaultChoice {
    /// 选为默认的提供方插件 ID。
    pub provider: String,
}

/// 使用方插件详情里一种能力的提供方选择（ADR-0053）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConsumerCapabilitySetting {
    /// 能力名。
    pub capability: String,
    /// 是不是第三方定义的能力。
    pub third_party: bool,
    /// 装着的提供方，按装的先后。
    pub providers: Vec<CapabilityProviderRef>,
    /// 给它单独指定的提供方；空表示跟全局默认走。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub selected: Option<String>,
}

/// 改使用方插件对一种能力的提供方选择（ADR-0053）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConsumerCapabilityChoice {
    /// 能力名。
    pub capability: String,
    /// 选的提供方插件 ID；空表示跟全局默认走。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub provider: Option<String>,
}

/// 卸载预览（ADR-0053）：卸一个使用方后，哪些提供方再没有别的插件在用。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UninstallPreview {
    /// 卸载的插件 ID。
    pub id: String,
    /// 卸载它之后就没有任何插件在用的提供方；主人可以选择一起卸，核心不自动卸。
    pub unused_providers: Vec<CapabilityProviderRef>,
}

/// 卸载请求里要一起卸的提供方（ADR-0053）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UninstallRequest {
    /// 一起卸的提供方插件 ID。
    pub providers: Vec<String>,
}

/// 安装或升级预览里“装不了”的类别（ADR-0053）。比这个 App 新的核心给的类别落到 [`BlockedCategory::Other`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum BlockedCategory {
    /// 新版本不再提供某个有人 requires 的能力，又没有别的提供方能接上。
    CapabilityDropped,
    /// 比这个 App 新的核心给的类别。
    #[serde(other)]
    Other,
}
