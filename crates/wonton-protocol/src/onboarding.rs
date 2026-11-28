//! 新用户引导（ADR-0034）。
//!
//! 核心的管理接口收发前半部分的类型，文字由核心按请求的 `Accept-Language` 挑好语言；客户端核心算出
//! [`OnboardingViewModel`] 交给壳。

use serde::{Deserialize, Serialize};

use crate::{
    BotNameProblem, BotRef, CardLine, ModelTestResult, PermissionLine, PluginIcon, PluginProblem,
    StoreItem, StoreProblem,
};

/// 读写引导状态的接口：`GET` 和 `PUT` 都返回 [`Onboarding`]，`PUT` 的请求体是 [`OnboardingUpdate`]。
pub const ONBOARDING_PATH: &str = "/_wonton/api/onboarding";

/// 模型设置网页接口：`GET` 返回网关网页目标和已配置状态。
pub const MODEL_SETTINGS_PATH: &str = "/_wonton/api/models/settings";

/// 引导的一步。家里的服务器上只记 `Profile`（还没走完）和 `Finished`：每一步只记下选择，到 `Review` 点完成才一起
/// 生效（ADR-0038），所以中间的步骤只在客户端核心里。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum OnboardingStep {
    Profile,
    /// 你的助手：默认 Bot 的名字（和头像）。
    Assistant,
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
    /// 默认 Bot 插件：默认安装里的那个 Agent 插件。默认安装里没有 Agent 时为空。
    pub default_bot_plugin: Option<DefaultBotPlugin>,
    pub needs: Vec<OnboardingNeed>,
    /// 第一次和默认 Bot 聊天时可点的问题，按主人的语言。
    pub prompts: Vec<String>,
    /// 官方索引的推荐清单取不到或者可能不是最新的原因。
    pub catalog_problem: Option<StoreProblem>,
    /// 这个人见过 Agent 的第一次出场（ADR-0126）：按人存在服务器上，换手机不再演。不记这一项的服务器
    /// 不给，为空：不知道见没见过，不演。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub agent_intro: Option<bool>,
}

/// `PUT /_wonton/api/onboarding` 的请求体。没写的字段不变。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingUpdate {
    #[serde(default)]
    pub step: Option<OnboardingStep>,
    /// 写 `true` 记下见过出场；只会从没见过变成见过，重看引导也不清。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub agent_intro: Option<bool>,
}

/// 每个 Agent 插件装好就有的那个 Bot（插件规范 `bots` 里的名字）：默认 Bot 插件上的它就是默认 Bot。
pub const DEFAULT_BOT: &str = "default";

/// 默认 Bot 插件：家人的私人 Bot 装在它上面，“问 Bot”和聊天列表置顶都找它的默认 Bot。由随核心附带的默认插件
/// 定，主人不在引导里选；主人卸掉它以后仍是它，私人 Bot 如实报未安装。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DefaultBotPlugin {
    pub id: String,
    /// 已核实可见的默认 Bot 的 Matrix 用户 ID。
    pub bot_user_id: Option<String>,
    /// 默认 Bot 身份的投影；未提供时表示未知。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub visibility: Option<crate::DefaultBotVisibility>,
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

/// `GET /_wonton/api/models/settings` 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSettings {
    pub plugin_id: String,
    /// 模型网关的网页；网关没有网页时为空。
    pub web_url: Option<String>,
    /// 核心曾成功为至少一个使用方签发并验证过模型网关凭据；不表示默认模型可用或请求一定成功。
    pub has_configured_models: bool,
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
    pub assistant: OnboardingAssistantStep,
    pub model: OnboardingModelStep,
    pub needs: Vec<OnboardingNeedRow>,
    pub review: OnboardingReview,
    /// 默认 Bot 插件的默认 Bot：引导走完后进入和它的私聊，[`Self::chat_prompts`] 也只在这个私聊里显示。
    pub default_bot_user_id: Option<String>,
    /// 默认 Bot 身份的投影；未提供时表示未知，不代表房间发送资格。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub default_bot_visibility: Option<crate::DefaultBotVisibility>,
    /// 输入框上方可点的问题，点了放进输入框当草稿；发过第一条消息后为空。
    pub chat_prompts: Vec<String>,
    pub tips: OnboardingTips,
    /// 服务器太旧，引导整段打不开（连引导接口都没有）：[`Self::step`] 为空，壳只在引导该出现的地方留一条
    /// “升级服务器后可用”和服务器升级入口。
    pub too_old: Option<OnboardingTooOld>,
    /// Agent 的第一次出场；为空时不演。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub agent_intro: Option<AgentIntroView>,
}

/// Agent 会话页第一次打开时的出场（ADR-0126 的“第一次出场”）。见过标记跟着人存在服务器上
/// （[`Onboarding::agent_intro`]），动画演到哪不存。气泡的两句话和按钮的文字按类别写在壳里。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgentIntroView {
    /// 该演：默认 Agent 的私聊在、连着，这个人还没见过。壳只在 Agent 会话页刚切进来时演；
    /// 演到一半切走，回来重演。
    pub show: bool,
    /// 默认 Agent 的名字，第一句气泡叫它。
    pub name: String,
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
    /// 先是家里的服务器上记着的，主人改了以后是改后的（[`OnboardingAction::EditOnboardingProfile`](crate::OnboardingAction::EditOnboardingProfile)）。
    pub display_name: String,
    pub email: String,
    /// 点下一步时名字不合规则的原因。
    pub display_name_problem: Option<String>,
    /// 点下一步时邮箱不合规则的原因；没填不算。
    pub email_problem: Option<String>,
}

/// 你的助手这一步：给默认 Bot 起名字。只记下来，点完成时才保存。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingAssistantStep {
    /// 先是 Bot 现在的名字，主人改了以后是改后的（[`OnboardingAction::EditOnboardingAssistant`](crate::OnboardingAction::EditOnboardingAssistant)）。
    pub name: String,
    /// 点下一步时名字不合规则的原因。
    pub name_problem: Option<BotNameProblem>,
}

/// 接模型这一步：壳在这一步放默认 Bot 的模型页（[`AgentModelsAction::OpenAgentModels`](crate::AgentModelsAction::OpenAgentModels)）。
/// 模型页里的操作当场存进 Bot，不属于引导草稿；选好主模型才能点下一步。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingModelStep {
    /// 默认 Bot；没有默认 Bot 插件时这一步不显示。
    pub bot: Option<BotRef>,
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

/// 最后一页：列出所有选择和应用要的权限。点完成（[`OnboardingAction::ContinueOnboarding`](crate::OnboardingAction::ContinueOnboarding)）
/// 算主人确认了这些权限，先保存资料，应用在后台安装，每一项的进展都在这里。
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
    /// 在保存资料、助手的名字和引导状态；模型页里的操作当场存过，这里不重做。
    Running,
    /// 都保存好了，安装在后台接着做；可以离开引导（再发一次 `ContinueOnboarding`）。
    Done,
}

/// 最后一页的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OnboardingReviewItem {
    /// 重试时带回（[`OnboardingAction::RetryOnboardingItem`](crate::OnboardingAction::RetryOnboardingItem)）。应用是插件 ID。
    pub id: String,
    pub kind: OnboardingReviewKind,
    /// 资料是填的名字，助手是 Bot 的名字，模型是服务商和模型，应用是插件名。
    pub titles: Vec<String>,
    /// 第二行的内容，壳按类别写文字；没有时为空。
    pub detail: Option<OnboardingReviewDetail>,
    /// 要装的应用的图标。
    pub icon: Option<PluginIcon>,
    /// 要装的应用申请的权限，点完成就是授予它们。
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
    /// 默认 Bot 选好的模型（`titles` 是服务商和模型）和这次打开模型页以来最近一次测试的结果；没测过时为空。
    Model { test: Option<ModelTestResult> },
    /// 这一类这次不动：完成时不发请求，服务器上的现值保持原样。
    Unchanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum OnboardingReviewKind {
    Profile,
    Assistant,
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
            "default_bot_plugin": null,
            "needs": [],
            "prompts": [],
            "catalog_problem": null,
        }))
        .unwrap();
        assert_eq!(onboarding.step, OnboardingStep::Other);
        // 不记出场的服务器不给这一项：不知道，不当成没见过。
        assert_eq!(onboarding.agent_intro, None);
    }

    #[test]
    fn the_agent_intro_mark_travels_with_the_onboarding_state() {
        let seen: Onboarding = serde_json::from_value(json!({
            "step": "finished", "default_bot_plugin": null, "needs": [], "prompts": [],
            "catalog_problem": null, "agent_intro": true,
        }))
        .unwrap();
        assert_eq!(seen.agent_intro, Some(true));
        // 只改步骤的请求不带这一项，旧服务器看到的和以前一样。
        let step_only = OnboardingUpdate {
            step: Some(OnboardingStep::Finished),
            agent_intro: None,
        };
        assert_eq!(
            serde_json::to_value(step_only).unwrap(),
            json!({"step": "finished"})
        );
        let mark = OnboardingUpdate {
            step: None,
            agent_intro: Some(true),
        };
        assert_eq!(
            serde_json::to_value(mark).unwrap(),
            json!({"step": null, "agent_intro": true})
        );
        let action = crate::Action::Onboarding {
            action: crate::OnboardingAction::FinishAgentIntro,
        };
        assert_eq!(
            serde_json::to_value(&action).unwrap(),
            json!({"domain": "onboarding", "action": {"type": "finish_agent_intro"}})
        );
    }
}
