//! 新用户引导和模型服务商（ADR-0034）。
//!
//! 核心的管理接口收发前半部分的类型，文字由核心按请求的 `Accept-Language` 挑好语言；客户端核心算出
//! [`OnboardingViewModel`] 交给壳。

use serde::{Deserialize, Serialize};

use crate::{PluginProblem, PluginState, SecretString, StoreItem, StoreProblem};

/// 读写引导状态的接口：`GET` 和 `PUT` 都返回 [`Onboarding`]，`PUT` 的请求体是 [`OnboardingUpdate`]。
pub const ONBOARDING_PATH: &str = "/_wonton/api/onboarding";

/// 服务商列表的接口：`GET` 返回 [`ModelProviders`]。
pub const MODEL_PROVIDERS_PATH: &str = "/_wonton/api/models/providers";

/// 保存一个服务商的 key 的接口：`PUT`，请求体是 [`ModelProviderKey`]，成功时没有响应体。错误类别：
/// `invalid_input` 是服务商拒绝了 key，这时不保留它；`upstream_unavailable` 是连不上服务商，key 已保存；
/// `models_unavailable` 是模型网关没就绪。
pub fn model_provider_path(id: &str) -> String {
    format!("{MODEL_PROVIDERS_PATH}/{id}")
}

/// 引导接下来要做的一步。记在核心上，第二台设备不重复。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum OnboardingStep {
    Profile,
    Agent,
    Model,
    Apps,
    Finished,
    /// 比这个 App 新的核心给的步骤。
    #[serde(other)]
    Other,
}

/// `GET /_wonton/api/onboarding` 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Onboarding {
    pub step: OnboardingStep,
    /// 默认 Agent 的插件 ID。只决定聊天列表里排第一的是谁，核心不按它路由。
    pub primary_agent: Option<String>,
    /// 已经问过要不要卸掉被换下来的 Agent，只问一次。
    pub replaced_agent_asked: bool,
    /// 官方索引推荐的 Agent，加上已经装着的。
    pub agents: Vec<OnboardingAgent>,
    pub needs: Vec<OnboardingNeed>,
    /// 第一次和默认 Agent 聊天时可点的问题，按主人的语言。
    pub prompts: Vec<String>,
    /// 官方索引的推荐清单取不到或者可能不是最新的原因。
    pub catalog_problem: Option<StoreProblem>,
}

/// `PUT /_wonton/api/onboarding` 的请求体。没写的字段不变。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingUpdate {
    #[serde(default)]
    pub step: Option<OnboardingStep>,
    #[serde(default)]
    pub primary_agent: Option<String>,
    #[serde(default)]
    pub replaced_agent_asked: Option<bool>,
}

/// 能选的一个 Agent。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingAgent {
    pub id: String,
    pub title: String,
    /// 商店给的安装来源；已经装着、不在官方索引里的 Agent 为空。
    pub source: Option<String>,
    /// 装着时是它的状态；没装时为空。
    pub status: Option<PluginState>,
    /// 插件的 Bot（Matrix 用户 ID）；没装时为空。
    pub bot_user_id: Option<String>,
    /// 官方程序自己的账号登录（清单的 `agent.login`）。为空时只能接模型网关。
    pub logins: Vec<AgentLogin>,
    /// 装不了的原因；能装时为空。
    pub blocked: Option<String>,
}

/// 官方程序自己的一种账号登录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgentLogin {
    pub id: String,
    /// 如“ChatGPT 账号”。
    pub title: String,
}

/// 一项需求和它推荐的插件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingNeed {
    pub id: String,
    pub title: String,
    /// 已经装的看 [`StoreItem::installed_version`]，这台主机装不了的看 [`StoreItem::blocked`]。
    pub plugins: Vec<StoreItem>,
}

/// `GET /_wonton/api/models/providers` 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelProviders {
    /// 推荐的在前。模型网关没有声明 `providers` 时为空，这时打开 `web_url`。
    pub providers: Option<Vec<ModelProvider>>,
    /// 模型网关的网页，用于“更多服务商”；网关没有网页时为空。
    pub web_url: Option<String>,
}

/// 模型网关能接的一个服务商。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelProvider {
    pub id: String,
    /// 厂商的名字，不分语言。
    pub title: String,
    /// 拿 key 的地址；不在推荐清单里的为空。
    pub key_url: Option<String>,
    /// 在主人语言的推荐清单里。
    pub recommended: bool,
    /// 模型网关里已经有它。
    pub configured: bool,
}

/// `PUT /_wonton/api/models/providers/{id}` 的请求体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelProviderKey {
    pub key: SecretString,
}

/// 引导的 ViewModel。步骤的判断都在客户端核心，壳只渲染，操作发回核心。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingViewModel {
    /// 还没从家里的服务器取到时为空，这时不知道引导走完了没有。
    pub step: Option<OnboardingStep>,
    /// 当前步骤的标题。
    pub title: String,
    /// 正在从家里的服务器取状态。
    pub loading: bool,
    /// 最近一次取状态或者保存进度失败的原因。
    pub problem: Option<PluginProblem>,
    /// 官方索引的推荐清单取不到或者可能不是最新的原因：这一页说明它，可以跳过。
    pub catalog_problem: Option<StoreProblem>,
    pub agents: Vec<OnboardingAgentRow>,
    pub model: OnboardingModelStep,
    pub needs: Vec<OnboardingNeedRow>,
    /// 默认 Agent 的 Bot：引导走完后进入和它的私聊，[`Self::chat_prompts`] 也只在这个私聊里显示。
    pub primary_bot_user_id: Option<String>,
    /// 输入框上方可点的问题，点了放进输入框当草稿；发过第一条消息后为空。
    pub chat_prompts: Vec<String>,
    /// 默认 Agent 能聊之后，问一次要不要卸掉被换下来的 Agent。卸载走原有流程，回答后发
    /// [`Action::DismissReplacedAgentOffer`](crate::Action::DismissReplacedAgentOffer)。
    pub replaced_agent: Option<ReplacedAgent>,
    pub tips: OnboardingTips,
}

/// 选 Agent 这一步的一行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingAgentRow {
    pub id: String,
    pub title: String,
    /// 接模型的方式，如“ChatGPT 账号或 key”。
    pub connection: String,
    pub selected: bool,
    /// 要装时的安装来源，走现有的确认页；已经装着时为空。
    pub source: Option<String>,
    /// 装不了的原因；能装时为空。
    pub blocked: Option<String>,
}

/// 接模型这一步。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingModelStep {
    /// 还没取到时为空。
    pub providers: Option<ModelProviders>,
    /// 选中的 Agent 的账号登录。选了就发 [`Action::ContinueOnboarding`](crate::Action::ContinueOnboarding)，登录链接由
    /// Agent 发进聊天。
    pub logins: Vec<AgentLogin>,
    /// 正在保存和测试 key 的服务商。
    pub saving: Option<String>,
    pub problem: Option<ModelProviderProblem>,
}

/// 保存 key 或者取服务商列表失败了。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelProviderProblem {
    pub reason: ModelProviderFailure,
    /// 核心写的说明，壳原样显示。
    pub message: String,
}

/// 失败的原因，壳据此提示主人下一步。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ModelProviderFailure {
    /// 服务商拒绝了 key，没有保留。
    KeyRejected,
    /// 连不上服务商，key 已保存。
    ProviderUnreachable,
    /// 模型网关没就绪。
    GatewayNotReady,
    /// 连不上家里的服务器。
    TransportUnavailable,
    /// 家里的服务器不认这台设备，或者不让它做这件事。
    Unauthorized,
    /// 家里的服务器上出错了。
    Upstream,
    /// App 和家里服务器的版本对不上，有一边要更新。
    Protocol,
}

/// 选需求这一步的一行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingNeedRow {
    pub id: String,
    pub title: String,
    pub selected: bool,
    pub plugins: Vec<StoreItem>,
}

/// 可以卸掉的 Agent。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ReplacedAgent {
    pub id: String,
    pub title: String,
}

/// 在用的时候教的提示，每台设备一次。有文字时壳在对应的地方显示。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingTips {
    /// 第一次出现确认卡片的提示条时。
    pub confirmation_card: Option<String>,
    /// 第一次走到应用的完成页时。
    pub app_installed: Option<String>,
}

/// 哪一条提示。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum OnboardingTip {
    ConfirmationCard,
    AppInstalled,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_step_from_a_newer_core_does_not_break_the_response() {
        let onboarding: Onboarding = serde_json::from_value(json!({
            "step": "from_the_future",
            "primary_agent": null,
            "replaced_agent_asked": false,
            "agents": [],
            "needs": [],
            "prompts": [],
            "catalog_problem": null,
        }))
        .unwrap();
        assert_eq!(onboarding.step, OnboardingStep::Other);
    }

    #[test]
    fn an_update_leaves_out_what_it_does_not_change() {
        let update: OnboardingUpdate = serde_json::from_value(json!({ "step": "model" })).unwrap();
        assert_eq!(update.step, Some(OnboardingStep::Model));
        assert_eq!(update.primary_agent, None);
        assert_eq!(update.replaced_agent_asked, None);
    }

    #[test]
    fn a_gateway_without_providers_is_not_an_empty_list() {
        let none: ModelProviders =
            serde_json::from_value(json!({ "providers": null, "web_url": null })).unwrap();
        let empty: ModelProviders =
            serde_json::from_value(json!({ "providers": [], "web_url": null })).unwrap();
        assert_ne!(none, empty);
    }

    #[test]
    fn a_provider_key_is_redacted_when_logged() {
        let key = ModelProviderKey {
            key: SecretString::new("sk-secret"),
        };
        assert!(!format!("{key:?}").contains("sk-secret"));
        assert_eq!(
            serde_json::to_value(&key).unwrap(),
            json!({ "key": "sk-secret" })
        );
    }

    #[test]
    fn provider_paths_sit_under_the_list() {
        assert_eq!(
            model_provider_path("deepseek"),
            "/_wonton/api/models/providers/deepseek"
        );
    }
}
