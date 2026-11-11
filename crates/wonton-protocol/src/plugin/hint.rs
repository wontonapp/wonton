//! 详情页给主人的轻提示：插件缺了某个平台能力、在用它自己的设置时提个醒。不是错误，插件状态照常。
//! 类别由核心给，App 按类别本地化（ADR-0043）；核心写的文字只有“详情”，App 收进默认收起的一节。

use serde::{Deserialize, Serialize};

use super::capability_deps::CapabilityProviderRef;

/// 详情页的一条轻提示。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginHint {
    pub category: PluginHintCategory,
    /// 核心按 App 的语言写的补充说明（如签发失败的原因）；没有时为空。
    #[serde(default)]
    pub detail: Option<String>,
    /// `dependency_stopped`：没有在运行的提供方（ID 和标题，ADR-0053）。文字由壳按类别写，要标题才写得出名字。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub provider: Option<CapabilityProviderRef>,
    /// `wants_unsatisfied`：没有提供方的能力名（ADR-0053）。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub capability: Option<String>,
}

/// 轻提示的类别。壳按类别写文字；不认识的类别是比这个 App 新的核心给的，照 [`PluginHintCategory::Other`]
/// 处理，显示与否由壳定。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PluginHintCategory {
    /// 没接模型网关：没装、没就绪，或给这个插件的凭据签发/验证失败——声明 `models` 权限的插件拿不到
    /// `WONTON_MODEL_*` 变量，在用它自己的登录或 key 设置。
    ModelsNotConnected,
    /// 接上了模型网关，但默认路由里没有模型：主人在网关（模型设置）里选服务商或加模型。
    ModelsNoDefaultModel,
    /// 默认模型的服务商出错（如 key 被拒）：`detail` 是核心按 App 语言写的出错原因。
    ModelsProviderError,
    /// 依赖的插件（`requires` 的能力，ADR-0053）没有在运行：`provider` 是提供方的 ID 和标题。
    DependencyStopped,
    /// `wants` 的能力有了提供方、但还没给这个插件接上（ADR-0053 的“高权限能力”，提供方后装时没勾
    /// 接上）：`provider` 是提供方，`capability` 是能力名。以后可以在这条提示处接上。
    ProviderNotConnected,
    /// `wants` 的能力没有提供方：`capability` 是能力名（ADR-0053）。
    WantsUnsatisfied,
    /// 不认识的类别。
    #[serde(other)]
    Other,
}
