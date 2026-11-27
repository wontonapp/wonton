//! 一个 Bot 的“模型”页（ADR-0129）：客户端核心经 [`crate::agent_models_path`] 等路由取数据、做操作，壳只画这份
//! ViewModel、把点击发回 [`AgentModelsAction`]，并按 [`ModelLoginProgress`] 打开 App 内的浏览器。
//!
//! 一次看一个 Bot。服务商的名单、顺序和接法全来自插件，这里只分“推荐”和“更多”。

use serde::{Deserialize, Serialize};

use crate::{
    BaseUrlField, BotRef, KeyCheck, ModelChoice, ModelLoginState, ModelOption, ModelTest,
    SecretString,
};

/// 模型页。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgentModelsViewModel {
    /// 正在看的 Bot（[`AgentModelsAction::OpenAgentModels`] 选的）；还没打开过时为空。
    pub bot: Option<BotRef>,
    pub state: AgentModelsState,
    /// 主模型；没选时为空。选了不代表能用，看 [`Self::test`]。
    pub current: Option<ModelChoice>,
    /// 这次打开页面以来最近一次测试；换了模型就清掉。
    pub test: Option<ModelTestState>,
    /// 插件标了推荐的服务商，插件给的顺序。
    pub recommended: Vec<ModelProviderRow>,
    /// 其余服务商，按 [`Self::search`] 过滤过（名字或 ID 含搜索词，不分大小写）。
    pub more: Vec<ModelProviderRow>,
    pub search: String,
    /// 点开的那家；没点开时为空。
    pub provider: Option<ModelProviderPage>,
    /// 上一个动作没成的原因，页面照旧显示；下一个动作清掉。
    pub action_problem: Option<AgentModelsProblem>,
}

/// 取服务商列表的状态。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentModelsState {
    #[default]
    Loading,
    Ready,
    Failed {
        problem: AgentModelsProblem,
    },
}

/// 列表里的一家。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelProviderRow {
    pub id: String,
    pub name: String,
    /// 填了 key 或登录了；不代表 key 有效。
    pub connected: bool,
    /// 登录的账号名，或 key 的尾号预览。
    pub account: Option<String>,
}

/// 点开的一家：按插件给的接法摆按钮。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelProviderPage {
    pub row: ModelProviderRow,
    /// 能填 key 时的输入框；不能填时为空。
    pub key: Option<ModelKeyForm>,
    /// 能用设备码登录。
    pub device_login: bool,
    /// 能在浏览器里授权后回跳 App。
    pub browser_login: bool,
    /// 正在登录；没在登录时为空。
    pub login: Option<ModelLoginProgress>,
    /// 上一次登录怎么结束的（被拒、过期、出错）；登好了或又开始登录时清掉。
    pub login_outcome: Option<ModelLoginState>,
    /// 正在存 key、断开或选模型，按钮先禁用。
    pub busy: bool,
    /// 最近一次存 key 的结果。
    pub key_check: Option<KeyCheck>,
    pub models: ModelListState,
    /// 插件提示这个模型贵，要点确认才保存；没在等确认时为空。
    pub confirm: Option<ModelConfirm>,
}

/// 填 key 的输入框。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelKeyForm {
    /// 去哪拿 key，在 App 内的浏览器里打开；只会是 `https://`。
    pub key_url: Option<String>,
    /// key 的固定开头：Android 回到前台时据此认剪贴板里刚复制的 key。
    pub key_prefix: Option<String>,
    /// 要填服务地址时的输入框。
    pub base_url: Option<BaseUrlField>,
}

/// 一次登录的进展。`url` 在 App 内的浏览器里打开；登好或失败后这一项清空，壳随之关掉浏览器。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelLoginProgress {
    /// 只会是 `https://`。
    pub url: String,
    /// 设备码；网址里没带时壳大字显示、可复制。
    pub code: Option<String>,
    pub browser: ModelLoginBrowser,
}

/// 用哪种浏览器打开登录页。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ModelLoginBrowser {
    /// 设备码登录：普通的 App 内浏览器（`SFSafariViewController`、Custom Tabs），核心轮询到批准就清掉登录，壳关浏览器。
    Page,
    /// 浏览器授权后回跳 [`MODEL_LOGIN_REDIRECT_URI`](crate::MODEL_LOGIN_REDIRECT_URI)：`ASWebAuthenticationSession`、
    /// Auth Tab，拿到回跳网址发 [`AgentModelsAction::FinishModelLogin`]。
    Redirect,
    #[serde(other)]
    Other,
}

/// 这家的模型列表。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelListState {
    /// 还没接好，不取。
    #[default]
    NotConnected,
    Loading,
    Ready {
        models: Vec<ModelOption>,
    },
    Failed {
        problem: AgentModelsProblem,
    },
}

/// 等确认的贵模型。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelConfirm {
    pub model: String,
    /// 插件给的一句说明，原样显示。
    pub message: String,
}

/// 测试的进展。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelTestState {
    Running,
    Done { result: ModelTest },
    Failed { problem: AgentModelsProblem },
}

/// 一次失败。壳按 `reason` 写一句人话，`message` 和 `detail` 收进默认收起的“详情”。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgentModelsProblem {
    pub reason: AgentModelsReason,
    pub message: String,
    /// `invalid_input` 时哪一项不对（如 `key`、`base_url`）。
    pub field: Option<String>,
    pub detail: Option<String>,
}

/// 失败的类别：核心给的 [`AgentModelsFailure`](crate::AgentModelsFailure)，加上客户端自己判断的连不上和读不懂。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AgentModelsReason {
    /// 连不上家里的服务器。
    Transport,
    /// 没有权限，或这个 Bot 不归他管。
    Forbidden,
    /// 没装这个 Agent，或它里面没有这个 Bot。
    NotInstalled,
    /// Bot 还没起来。
    NotReady,
    /// 这个 Agent 不能在 App 里接模型。
    Unsupported,
    /// 服务商或登录已经不在了。
    NotFound,
    /// 填的内容不对。
    InvalidInput,
    /// Agent 处理时出错。
    Upstream,
    /// App 和家里的服务器版本对不上。
    Protocol,
    #[serde(other)]
    Other,
}

/// 模型页的操作。结果都落在 [`AgentModelsViewModel`]。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentModelsAction {
    /// 打开这个 Bot 的模型页或下拉刷新。换了 Bot 时清掉上一个的一切。
    OpenAgentModels {
        bot: BotRef,
    },
    SearchModelProviders {
        text: String,
    },
    OpenModelProvider {
        provider: String,
    },
    /// 关掉点开的那家；在登录就取消登录。
    CloseModelProvider,
    SaveModelKey {
        key: SecretString,
        base_url: Option<String>,
    },
    DisconnectModelProvider,
    /// 按点开那家的接法开始登录：能回跳的走浏览器授权，否则走设备码。
    StartModelLogin,
    /// 浏览器授权回跳到 App 的完整网址。
    FinishModelLogin {
        callback_url: String,
    },
    /// 关了浏览器、点了取消。
    CancelModelLogin,
    /// 在点开那家里选这个模型。
    SelectModel {
        model: String,
    },
    /// 看过贵模型的提示，确认用它。
    ConfirmModelSelection,
    DismissModelConfirmation,
    TestModel,
}
