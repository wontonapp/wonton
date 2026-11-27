//! Agent 插件的模型接入（ADR-0129）：App 原生地列服务商、填 key、登录、选模型和测试。
//!
//! 两段接口用同一套数据：
//!
//! - **核心 ↔ 插件**：清单 `agent.models` 声明的命令（[`crate::manifest::AgentModels`]），核心经 Docker exec 在这个
//!   Bot 所在的容器里执行，标准输入一行 [`ModelsCommandInput`]，标准输出一行对应操作的结果。退出码见 [`EXIT_INVALID_INPUT`]
//!   等常量；服务商拒了 key、登录被拒、测试不通都不是命令失败，退出码是 0，结果在字段里。
//! - **App ↔ 核心**：按 Bot 的路由（[`agent_models_path`] 等），请求体和响应就是下面这些类型；出错时是核心统一的
//!   `{"error": {"kind", "message", "field"?}}`，`kind` 取 [`AgentModelsFailure`] 的值。
//!
//! 服务商名单、顺序、取 key 的网址全由插件给，核心和 App 不带服务商名单。核心可能比 App 新：多给的字段被忽略，
//! 不认识的枚举值落到 `Other`。

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};

use crate::SecretString;

/// `browser_login` 回跳 App 的地址：App 按它的 scheme 等回跳，插件拿它拼授权网址。不是 [`crate::AppLink`]。
pub const MODEL_LOGIN_REDIRECT_URI: &str = "wonton://model-login";

/// 命令的退出码：输入不合法，标准输出是 [`ModelsInvalidInput`]。
pub const EXIT_INVALID_INPUT: i32 = 2;
/// 命令的退出码：插件不认识这个服务商或登录会话（会话过期也算）。
pub const EXIT_UNKNOWN_TARGET: i32 = 3;
/// 命令的退出码：Agent 自己还没起来，接口不通。其余非 0 退出码一律是上游出错。
pub const EXIT_NOT_READY: i32 = 4;

/// `test` 最多等多久（秒）；它要真发一条请求。
pub const MODELS_TEST_TIMEOUT_SECS: u64 = 60;
/// 其余操作最多等多久（秒）。
pub const MODELS_TIMEOUT_SECS: u64 = 30;
/// 命令一次输出的上限，超了算上游出错。
pub const MAX_MODELS_OUTPUT_BYTES: usize = 1024 * 1024;
/// 一份目录最多几家服务商、一家最多几个模型。
pub const MAX_MODEL_PROVIDERS: usize = 500;
pub const MAX_PROVIDER_MODELS: usize = 5000;
/// 插件给的 ID、名字、一句话说明的长度上限（Unicode 标量）。
pub const MAX_MODELS_ID_CHARS: usize = 200;
pub const MAX_MODELS_TEXT_CHARS: usize = 500;
/// 登录轮询的间隔下限和上限（秒）：插件给的值落到这个范围里。
pub const MIN_LOGIN_POLL_SECS: u32 = 1;
pub const MAX_LOGIN_POLL_SECS: u32 = 30;

/// `GET`：这个 Bot 的 [`ModelCatalog`]。插件没声明 `agent.models` 时是 `unsupported`。
pub fn agent_models_path(plugin_id: &str, bot: &str) -> String {
    format!("/_wonton/api/bots/{}/{}/models", seg(plugin_id), seg(bot))
}

/// `PUT` 请求体 [`SaveModelKey`]，返回 [`KeyCheck`]；`DELETE` 断开这家（填的 key 或登录都去掉），返回 204。
pub fn agent_model_provider_path(plugin_id: &str, bot: &str, provider: &str) -> String {
    format!(
        "{}/providers/{}",
        agent_models_path(plugin_id, bot),
        seg(provider)
    )
}

/// `GET`：这家的 [`ModelList`]。
pub fn agent_model_list_path(plugin_id: &str, bot: &str, provider: &str) -> String {
    format!(
        "{}/models",
        agent_model_provider_path(plugin_id, bot, provider)
    )
}

/// `POST` 请求体 [`StartModelLogin`]，返回 [`ModelLogin`]。
pub fn agent_model_login_start_path(plugin_id: &str, bot: &str, provider: &str) -> String {
    format!(
        "{}/login",
        agent_model_provider_path(plugin_id, bot, provider)
    )
}

/// `GET` 轮询，返回 [`ModelLoginState`]；`DELETE` 取消，返回 204。
pub fn agent_model_login_path(plugin_id: &str, bot: &str, session: &str) -> String {
    format!(
        "{}/logins/{}",
        agent_models_path(plugin_id, bot),
        seg(session)
    )
}

/// `POST` 请求体 [`FinishModelLogin`]，返回 [`ModelLoginState`]。只给 `browser_login`。
pub fn agent_model_login_finish_path(plugin_id: &str, bot: &str, session: &str) -> String {
    format!("{}/finish", agent_model_login_path(plugin_id, bot, session))
}

/// `PUT` 请求体 [`SelectModel`]，返回 [`ModelSelection`]。
pub fn agent_model_current_path(plugin_id: &str, bot: &str) -> String {
    format!("{}/current", agent_models_path(plugin_id, bot))
}

/// `POST`，没有请求体，返回 [`ModelTest`]。会真发一条很短的请求，可能计费。
pub fn agent_model_test_path(plugin_id: &str, bot: &str) -> String {
    format!("{}/test", agent_models_path(plugin_id, bot))
}

/// 路径段里除了 RFC 3986 的非保留字符都编码：插件 ID 的点照原样，服务商 ID 里的 `:`、`/` 编进一段。
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn seg(value: &str) -> String {
    utf8_percent_encode(value, SEGMENT).to_string()
}

/// 核心交给插件命令的一行输入。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelsCommandInput {
    #[serde(flatten)]
    pub op: ModelsOp,
    /// 哪个 Bot（插件规范 `bots` 里的 Bot 名）。
    pub bot: String,
    /// App 的语言，从最想要的排起（BCP 47）。插件写给人看的那一句按它挑。
    pub languages: Vec<String>,
}

/// 一次操作和它的参数。`key` 是 [`SecretString`]，打日志不会露出来。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ModelsOp {
    Catalog,
    SaveKey {
        provider: String,
        key: SecretString,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
    },
    Disconnect {
        provider: String,
    },
    LoginStart {
        provider: String,
        /// 只给 `browser_login`：[`MODEL_LOGIN_REDIRECT_URI`]。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        redirect_uri: Option<String>,
    },
    LoginPoll {
        session: String,
    },
    LoginFinish {
        session: String,
        callback_url: String,
    },
    LoginCancel {
        session: String,
    },
    Models {
        provider: String,
    },
    Select {
        provider: String,
        model: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        confirmed: bool,
    },
    Test,
}

/// 命令退出 [`EXIT_INVALID_INPUT`] 时的输出，核心原样转成 `invalid_input`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelsInvalidInput {
    /// 哪个字段不对，如 `key`、`base_url`；说不清时为空。
    #[serde(default)]
    pub field: Option<String>,
    pub message: String,
}

/// `catalog` 的结果。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelCatalog {
    /// 插件给的顺序。
    pub providers: Vec<ModelProvider>,
    /// 主模型；没选时为空。选了不代表能用，能不能用看 [`ModelTest`]。
    #[serde(default)]
    pub current: Option<ModelChoice>,
}

/// 一家服务商。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelProvider {
    pub id: String,
    pub name: String,
    /// 插件建议放在前面的；其余进“更多”。
    #[serde(default)]
    pub recommended: bool,
    /// 能怎么接，插件给的顺序。一种都不认识时壳不列这家。
    pub auth: Vec<ModelAuth>,
    /// 填了 key 或登录了。不代表 key 有效。
    #[serde(default)]
    pub connected: bool,
    /// 登录的账号名，或 key 的尾号预览；不会是完整的 key。
    #[serde(default)]
    pub account: Option<String>,
}

/// 一种接法。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelAuth {
    /// 填 key。
    ApiKey {
        /// 去哪拿 key，只收 `https://`。
        #[serde(default)]
        key_url: Option<String>,
        /// key 的固定开头，如 `sk-or-`：Android 回到前台时看剪贴板是不是刚复制的 key。
        #[serde(default)]
        key_prefix: Option<String>,
        /// 要不要填服务地址（自定义的 OpenAI 兼容服务商）；不要时为空。
        #[serde(default)]
        base_url: Option<BaseUrlField>,
    },
    /// 设备码登录：App 打开 [`ModelLogin::url`]、显示码，轮询到批准为止。
    DeviceLogin,
    /// 浏览器授权后回跳 [`MODEL_LOGIN_REDIRECT_URI`]。
    BrowserLogin,
    #[serde(other)]
    Other,
}

/// 服务地址的输入框。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BaseUrlField {
    pub required: bool,
    #[serde(default)]
    pub default: Option<String>,
}

/// 选中的模型。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelChoice {
    pub provider: String,
    pub model: String,
}

/// `PUT` [`agent_model_provider_path`] 的请求体。`key` 不回显、不进日志。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveModelKey {
    pub key: SecretString,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

/// `save_key` 的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct KeyCheck {
    pub check: KeyCheckResult,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum KeyCheckResult {
    /// 服务商认了这把 key，已保存。
    Accepted,
    /// 服务商拒了，没保存。
    Rejected,
    /// 这家没法提前验证，照存了；能不能用要测试。
    Unverified,
    #[serde(other)]
    Other,
}

/// `POST` [`agent_model_login_start_path`] 的请求体。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartModelLogin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
}

/// `login_start` 的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelLogin {
    pub session: String,
    /// 在 App 里打开的网址，只收 `https://`。设备码登录时尽量带上码。
    pub url: String,
    /// 设备码；网址里没带码时 App 大字显示、可复制。
    #[serde(default)]
    pub code: Option<String>,
    /// 多少秒后这次登录作废。
    pub expires_in: u32,
    /// 隔几秒轮询一次。
    pub interval: u32,
}

/// `POST` [`agent_model_login_finish_path`] 的请求体：回跳到 App 的完整网址。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinishModelLogin {
    pub callback_url: String,
}

/// `login_poll`、`login_finish` 的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelLoginState {
    pub state: LoginState,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum LoginState {
    Pending,
    /// 登好了，凭据已存进这个 Bot。
    Approved,
    /// 在服务商那边点了拒绝。
    Denied,
    Expired,
    Failed,
    #[serde(other)]
    Other,
}

/// `models` 的结果。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelList {
    pub models: Vec<ModelOption>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelOption {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// 插件推荐的，排第一、预选。
    #[serde(default)]
    pub recommended: bool,
}

/// `PUT` [`agent_model_current_path`] 的请求体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectModel {
    pub provider: String,
    pub model: String,
    /// 看过插件的贵模型提示、点了确认。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub confirmed: bool,
}

/// `select` 的结果：要确认时 `confirm` 是插件给的一句说明，这次没保存。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelSelection {
    #[serde(default)]
    pub current: Option<ModelChoice>,
    #[serde(default)]
    pub confirm: Option<String>,
}

/// `test` 的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelTest {
    pub result: ModelTestResult,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub latency_ms: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ModelTestResult {
    /// 真有回复。
    Ok,
    /// 服务商不认凭据（key 失效、登录过期）。
    Rejected,
    /// 连不上服务商。
    Unreachable,
    /// 服务商出错（额度用完、模型不存在等）。
    Upstream,
    /// 还没选模型。
    NoModel,
    #[serde(other)]
    Other,
}

/// 核心返回的错误类别（`error.kind`）。连不上家里由客户端核心自己判断，不在这里。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AgentModelsFailure {
    /// 没有 `model_settings` 权限，或这个 Bot 不归他管。
    Forbidden,
    /// 没装这个插件，或插件里没有这个 Bot。
    NotInstalled,
    /// 容器没在跑，或命令退出 [`EXIT_NOT_READY`]。
    NotReady,
    /// 插件没声明 `agent.models`。
    Unsupported,
    /// 插件不认识这个服务商或登录会话。
    NotFound,
    /// 输入不合法，`field` 说哪一项。
    InvalidInput,
    /// 命令失败、超时或输出不合格。
    Upstream,
    #[serde(other)]
    Other,
}

/// 插件给的网址能不能在 App 里打开：只收 `https://`，不收别的 scheme，防止插件让 App 打开任意协议。
pub fn is_browser_url(url: &str) -> bool {
    url.strip_prefix("https://").is_some_and(|rest| {
        !rest.is_empty()
            && !rest.starts_with('/')
            && !url.chars().any(|c| c.is_whitespace() || c.is_control())
    })
}

/// 插件输出哪里不合格。核心据此把这次调用算上游出错。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelsOutputProblem(pub String);

fn text(field: &str, value: &str, max: usize) -> Result<(), ModelsOutputProblem> {
    if value.trim().is_empty() || value.chars().count() > max || value.chars().any(char::is_control)
    {
        return Err(ModelsOutputProblem(format!(
            "{field} 为空、超长或带控制字符"
        )));
    }
    Ok(())
}

fn optional_text(field: &str, value: Option<&str>) -> Result<(), ModelsOutputProblem> {
    value.map_or(Ok(()), |value| text(field, value, MAX_MODELS_TEXT_CHARS))
}

impl ModelCatalog {
    /// 核心转给 App 之前查一遍：数量、长度、网址。不合格的整份不要，不挑着删。
    pub fn check(&self) -> Result<(), ModelsOutputProblem> {
        if self.providers.len() > MAX_MODEL_PROVIDERS {
            return Err(ModelsOutputProblem("服务商太多".into()));
        }
        let mut seen = std::collections::BTreeSet::new();
        for provider in &self.providers {
            text("providers.id", &provider.id, MAX_MODELS_ID_CHARS)?;
            text("providers.name", &provider.name, MAX_MODELS_TEXT_CHARS)?;
            optional_text("providers.account", provider.account.as_deref())?;
            if !seen.insert(provider.id.as_str()) {
                return Err(ModelsOutputProblem(format!("服务商 {} 重复", provider.id)));
            }
            for auth in &provider.auth {
                if let ModelAuth::ApiKey {
                    key_url,
                    key_prefix,
                    ..
                } = auth
                {
                    if key_url.as_deref().is_some_and(|url| !is_browser_url(url)) {
                        return Err(ModelsOutputProblem("key_url 不是 https 网址".into()));
                    }
                    optional_text("key_prefix", key_prefix.as_deref())?;
                }
            }
        }
        if let Some(current) = &self.current {
            current.check()?;
        }
        Ok(())
    }
}

impl ModelChoice {
    pub fn check(&self) -> Result<(), ModelsOutputProblem> {
        text("provider", &self.provider, MAX_MODELS_ID_CHARS)?;
        text("model", &self.model, MAX_MODELS_TEXT_CHARS)
    }
}

impl ModelList {
    pub fn check(&self) -> Result<(), ModelsOutputProblem> {
        if self.models.len() > MAX_PROVIDER_MODELS {
            return Err(ModelsOutputProblem("模型太多".into()));
        }
        for model in &self.models {
            text("models.id", &model.id, MAX_MODELS_TEXT_CHARS)?;
            optional_text("models.name", model.name.as_deref())?;
        }
        Ok(())
    }
}

impl ModelLogin {
    /// 网址只收 https；轮询间隔落到 [`MIN_LOGIN_POLL_SECS`]..=[`MAX_LOGIN_POLL_SECS`]。
    pub fn checked(mut self) -> Result<Self, ModelsOutputProblem> {
        text("session", &self.session, MAX_MODELS_ID_CHARS)?;
        if !is_browser_url(&self.url) {
            return Err(ModelsOutputProblem("登录网址不是 https 网址".into()));
        }
        optional_text("code", self.code.as_deref())?;
        self.interval = self
            .interval
            .clamp(MIN_LOGIN_POLL_SECS, MAX_LOGIN_POLL_SECS);
        Ok(self)
    }
}

#[cfg(test)]
#[path = "agent_models_tests.rs"]
mod tests;
