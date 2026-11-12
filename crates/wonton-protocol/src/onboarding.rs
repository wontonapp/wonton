//! 新用户引导和模型服务商（ADR-0034）。
//!
//! 核心的管理接口收发前半部分的类型，文字由核心按请求的 `Accept-Language` 挑好语言；客户端核心算出
//! [`OnboardingViewModel`] 交给壳。

use serde::{Deserialize, Serialize};

use crate::{
    CardLine, PermissionLine, PluginIcon, PluginProblem, PluginState, SecretString, StoreItem,
    StoreProblem,
};

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

/// 只测试一个服务商的 key、不保存的接口：`POST`，请求体是 [`ModelProviderKey`]，成功时没有响应体。错误类别同
/// [`model_provider_path`]，只是什么都不保存：`upstream_unavailable` 时 key 也没有保存。
pub fn model_provider_test_path(id: &str) -> String {
    format!("{MODEL_PROVIDERS_PATH}/{id}/test")
}

/// 保存一个 OpenAI 兼容的自定义服务商的接口：`PUT`，请求体是 [`CustomModelProvider`]。成功和错误同
/// [`model_provider_path`]。模型网关不支持自定义时是 `models_unavailable`，[`ModelProviders::custom`] 为假。
pub const CUSTOM_MODEL_PROVIDER_PATH: &str = "/_wonton/api/models/custom-provider";

/// 只测试自定义服务商、不保存的接口：`POST`，请求体是 [`CustomModelProvider`]，错误同 [`model_provider_test_path`]。
pub const CUSTOM_MODEL_PROVIDER_TEST_PATH: &str = "/_wonton/api/models/custom-provider/test";

/// 引导的一步。家里的服务器上只记 `Profile`（还没走完）和 `Finished`：每一步只记下选择，到 `Review` 点完成才一起
/// 生效（ADR-0038），所以中间的步骤只在客户端核心里。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum OnboardingStep {
    Profile,
    Agent,
    Model,
    Apps,
    /// 列出所有选择和应用要的权限，点完成才执行。
    Review,
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
    /// 官方索引列出的主流服务商，主人语言推荐的在前，其余的壳收进“更多服务商”。模型网关没有声明 `providers` 时为空；取不到
    /// 索引时是空列表。这两种都打开 `web_url`。
    pub providers: Option<Vec<ModelProvider>>,
    /// 模型网关里已经接了模型，包括不在 `providers` 里的。接模型这一步看它，不看列表里的 `configured`。
    #[serde(default)]
    pub any_configured: bool,
    /// 模型网关的网页；网关没有网页时为空。
    pub web_url: Option<String>,
    /// 模型网关里已经接好的服务商，包括不在 `providers` 里的（如自定义的）。重进引导时接模型这一步显示它们。
    #[serde(default)]
    pub configured: Vec<ConfiguredModelProvider>,
    /// 模型网关能接 OpenAI 兼容的自定义服务商（[`CUSTOM_MODEL_PROVIDER_PATH`]）。
    #[serde(default)]
    pub custom: bool,
}

/// 模型网关里已经接好的一个服务商。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConfiguredModelProvider {
    pub id: String,
    /// 网关给的名字；取不到时是 `id`。
    pub title: String,
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

/// `PUT /_wonton/api/models/providers/{id}` 和它的测试接口的请求体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelProviderKey {
    pub key: SecretString,
}

/// 自定义服务商：OpenAI 兼容接口的地址、key 和模型名。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomModelProvider {
    /// 如 `https://api.example.com/v1`。
    pub base_url: String,
    pub model: String,
    pub key: SecretString,
}

/// 接模型这一步填的东西，交给 [`Action::TestModelProvider`](crate::Action::TestModelProvider)。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelProviderInput {
    /// 列表里的服务商（[`ModelProvider::id`]）。
    Preset { id: String, key: SecretString },
    Custom {
        base_url: String,
        model: String,
        key: SecretString,
    },
}

/// 引导的 ViewModel。步骤的判断都在客户端核心，壳只渲染，操作发回核心。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingViewModel {
    /// 正在显示的一步：还没从家里的服务器取到、或者刚关掉重看的引导时为空。
    pub step: Option<OnboardingStep>,
    /// 第几步和哪些按钮能点；走完了或者还没取到时为空。
    pub nav: Option<StepNav>,
    /// 正在从家里的服务器取状态。
    pub loading: bool,
    /// 最近一次取状态或者保存进度失败的原因。
    pub problem: Option<PluginProblem>,
    /// 官方索引的推荐清单取不到或者可能不是最新的原因：这一页说明它，可以跳过。
    pub catalog_problem: Option<StoreProblem>,
    pub profile: OnboardingProfileStep,
    pub agents: Vec<OnboardingAgentRow>,
    pub model: OnboardingModelStep,
    pub needs: Vec<OnboardingNeedRow>,
    pub review: OnboardingReview,
    /// 默认 Agent 的 Bot：引导走完后进入和它的私聊，[`Self::chat_prompts`] 也只在这个私聊里显示。
    pub primary_bot_user_id: Option<String>,
    /// 输入框上方可点的问题，点了放进输入框当草稿；发过第一条消息后为空。
    pub chat_prompts: Vec<String>,
    /// 默认 Agent 能聊之后，问一次要不要卸掉被换下来的 Agent。卸载走原有流程，回答后发
    /// [`Action::DismissReplacedAgentOffer`](crate::Action::DismissReplacedAgentOffer)。
    pub replaced_agent: Option<ReplacedAgent>,
    pub tips: OnboardingTips,
    /// 服务器太旧，引导整段打不开（连引导接口都没有）：[`Self::step`] 为空，壳只在引导该出现的地方留一条
    /// “升级服务器后可用”和服务器升级入口。
    pub too_old: Option<OnboardingTooOld>,
}

/// 服务器太旧，引导整段用不了。各步单独隐藏的情况不在这里，见 [`OnboardingViewModel::step`]。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingTooOld {
    /// 服务器至少要升到的版本；旧服务器不报版本时为空。
    pub min_version: Option<String>,
}

/// 一步一张的外框：第几步、共几步，哪些按钮能点。壳的外框只认这个类型。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StepNav {
    /// 从 1 起。
    pub position: u32,
    pub total: u32,
    pub can_back: bool,
    pub can_skip: bool,
    pub can_next: bool,
    /// 从设置重看引导时每一步都能不提交就离开：壳画关闭（iOS 左上 X、Android 顶栏），
    /// 第一步的返回手势也当关闭。首次引导为假，照旧走完才算。
    pub can_close: bool,
}

/// 你的资料这一步。名字必填，邮箱选填。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingProfileStep {
    /// 先是家里的服务器上记着的，主人改了以后是改后的（[`Action::EditOnboardingProfile`](crate::Action::EditOnboardingProfile)）。
    pub display_name: String,
    pub email: String,
    /// 点下一步时名字不合规则的原因。
    pub display_name_problem: Option<String>,
    /// 点下一步时邮箱不合规则的原因；没填不算。
    pub email_problem: Option<String>,
}

/// 选 Agent 这一步的一行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingAgentRow {
    pub id: String,
    pub title: String,
    /// 能用来接模型的账号登录的名字，如“ChatGPT 账号”，服务器给的、已按语言挑好。空表表示只能用 key。
    pub logins: Vec<String>,
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
    /// 选中的 Agent 的账号登录。选了发 [`Action::ChooseModelLogin`](crate::Action::ChooseModelLogin)，登录链接由 Agent
    /// 发进聊天。
    pub logins: Vec<AgentLogin>,
    /// 这一步记下的选择，点完成时才保存；为空时不动模型。
    pub choice: Option<OnboardingModelChoice>,
    /// 正在测试填的 key，什么都不保存。
    pub testing: bool,
    pub problem: Option<ModelProviderProblem>,
}

/// 接模型这一步记下的选择。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OnboardingModelChoice {
    /// 用模型网关里已经接好的（[`ModelProviders::configured`]），点完成时不动。
    Current,
    /// 列表里的服务商，key 测过了。`reachable` 为假是连不上服务商：key 没被拒，点完成时照样保存。
    Provider {
        id: String,
        title: String,
        reachable: bool,
    },
    /// 自定义服务商，测过了。`reachable` 同上。
    Custom {
        base_url: String,
        model: String,
        reachable: bool,
    },
    /// 用 Agent 官方程序的账号登录（[`AgentLogin`]），登录链接由 Agent 发进聊天。
    Login { id: String, title: String },
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

/// 最后一页：列出所有选择和应用要的权限。点完成（[`Action::ContinueOnboarding`](crate::Action::ContinueOnboarding)）
/// 算主人确认了这些权限，资料、模型和默认 Agent 依次保存，Agent 和应用在后台安装，每一项的进展都在这里。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingReview {
    pub items: Vec<OnboardingReviewItem>,
    pub commit: OnboardingCommit,
}

/// 点完成之后到了哪里。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum OnboardingCommit {
    /// 还没点完成，什么都没有执行。
    #[default]
    NotStarted,
    /// 在保存资料、模型和引导状态。
    Running,
    /// 都保存好了，安装在后台接着做；可以离开引导（再发一次 `ContinueOnboarding`）。
    Done,
}

/// 最后一页的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingReviewItem {
    /// 重试时带回（[`Action::RetryOnboardingItem`](crate::Action::RetryOnboardingItem)）。Agent 和应用是插件 ID。
    pub id: String,
    pub kind: OnboardingReviewKind,
    /// 资料是填的名字，Agent 和应用是插件名，模型是服务商名（“用现在的”可能有几个）。壳用列表格式连起来。
    pub titles: Vec<String>,
    /// 第二行的内容，壳按类别写文字；没有时为空。
    pub detail: Option<OnboardingReviewDetail>,
    /// 要装的 Agent 和应用的图标。
    pub icon: Option<PluginIcon>,
    /// 要装的 Agent 和应用申请的权限，点完成就是授予它们。
    pub permissions: Vec<PermissionLine>,
    /// 确认前要看到的其他说明（未审核、镜像来源等），同安装确认页。
    pub notices: Vec<CardLine>,
    pub state: OnboardingItemState,
}

/// 最后一页一项的第二行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OnboardingReviewDetail {
    /// 资料的邮箱。
    Email { email: String },
    /// 已经装着的 Agent，完成后设成默认。
    SetAsDefault,
    /// 模型用模型网关里已经接好的。
    KeepCurrent,
    /// 自定义服务商测试通过时的模型名。
    Model { model: String },
    /// 测试时没连上服务商，照样保存。`model` 是自定义服务商的模型名，列表里的服务商为空。
    Unreachable { model: Option<String> },
    /// 选了 Agent 的账号登录，装好后在聊天里登录。
    SignInAfterSetup,
    /// 这一类这次不动：完成时不发请求，服务器上的现值保持原样。
    Unchanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum OnboardingReviewKind {
    Profile,
    /// 默认 Agent；没装时点完成会装上。
    Agent,
    Model,
    App,
}

/// 一项的进展。点完成之前只有 `Preparing`、`Ready`、`Blocked` 和 `Failed`；有一项不是 `Ready` 时完成不能点。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OnboardingItemState {
    /// 在取安装预览。
    Preparing,
    /// 点完成就做。
    Ready,
    /// 这台主机装不了，或者要先填资料（如邮箱）：回到那一步改，或者不选它。`message` 是原因：服务器给的原样放，
    /// 核心自己判断的（如权限变了）也放在这里；没有原因（没有安装来源）时为空，壳写“装不了”。
    Blocked {
        message: Option<String>,
    },
    /// 点了完成，排在前面的做完才轮到它。
    Waiting,
    Running,
    Done,
    /// 连接断了，等不到家里服务器的结果：它可能已经装好了，看已安装列表。
    Uncertain {
        message: String,
    },
    /// 失败了，可以重试；别的项照常做。
    Failed {
        problem: OnboardingItemProblem,
    },
}

/// 一项失败的原因，类别由核心给，壳按类别写文字（AGENTS.md“界面设计”）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OnboardingItemProblem {
    /// 资料、引导状态、安装预览和安装。
    Plugin { problem: PluginProblem },
    /// 保存模型。
    Model { problem: ModelProviderProblem },
}

/// 可以卸掉的 Agent。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ReplacedAgent {
    pub id: String,
    pub title: String,
}

/// 在用的时候教的提示，每台设备一次。为真时壳在对应的地方显示。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingTips {
    /// 第一次出现确认卡片的提示条时。
    pub confirmation_card: bool,
    /// 第一次走到应用的完成页时。
    pub app_installed: bool,
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
