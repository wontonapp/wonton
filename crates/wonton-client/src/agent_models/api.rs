//! 模型接入接口的请求：走本地代理，核心凭隧道身份认出这台设备和它的人（ADR-0017）。
//!
//! 失败的类别读回应体里的 `error.kind`；本地代理自己回的错误（隧道不通、设备被吊销）没有这个体，按状态码分。

use std::time::Duration;

use reqwest::header::{ACCEPT_LANGUAGE, CONTENT_TYPE};
use reqwest::{Client, Method};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{
    AgentModelsFailure, AgentModelsProblem, AgentModelsReason, BotRef, FinishModelLogin, KeyCheck,
    MODEL_LOGIN_REDIRECT_URI, MODELS_TEST_TIMEOUT_SECS, MODELS_TIMEOUT_SECS, ModelCatalog,
    ModelList, ModelLogin, ModelLoginState, ModelSelection, ModelTest, ModelsOutputProblem,
    ProxySettings, SaveModelKey, SecretString, SelectModel, StartModelLogin,
    agent_model_current_path, agent_model_list_path, agent_model_login_finish_path,
    agent_model_login_path, agent_model_login_start_path, agent_model_provider_path,
    agent_model_test_path, agent_models_path,
};

use crate::chat::proxy_url;
use crate::l10n::{CATALOG, language};

/// 核心那边命令的超时，加上隧道来回的余量：核心先超时，App 拿到的是核心分好类的“上游出错”，不是自己猜的“连不上”。
const MARGIN: Duration = Duration::from_secs(15);

fn timeout(test: bool) -> Duration {
    let secs = if test {
        MODELS_TEST_TIMEOUT_SECS
    } else {
        MODELS_TIMEOUT_SECS
    };
    Duration::from_secs(secs) + MARGIN
}

pub(super) fn problem(reason: AgentModelsReason, message: String) -> AgentModelsProblem {
    AgentModelsProblem {
        reason,
        message,
        field: None,
        detail: None,
    }
}

pub(super) fn transport(reason: String) -> AgentModelsProblem {
    problem(
        AgentModelsReason::Transport,
        l10n!(CATALOG, language(), "peers-transport", reason = reason),
    )
}

pub(super) fn not_connected() -> AgentModelsProblem {
    problem(
        AgentModelsReason::Transport,
        l10n!(CATALOG, language(), "peers-not-connected"),
    )
}

pub(super) fn unreadable(reason: String) -> AgentModelsProblem {
    problem(
        AgentModelsReason::Protocol,
        l10n!(
            CATALOG,
            language(),
            "peers-invalid-response",
            reason = reason
        ),
    )
}

fn unchecked(problem: ModelsOutputProblem) -> AgentModelsProblem {
    unreadable(problem.0)
}

pub(super) fn proxied_client(proxy: &ProxySettings) -> Result<Client, AgentModelsProblem> {
    let invalid = |e: reqwest::Error| {
        problem(
            AgentModelsReason::Protocol,
            l10n!(
                CATALOG,
                language(),
                "peers-invalid-proxy",
                reason = e.to_string()
            ),
        )
    };
    Client::builder()
        .pool_max_idle_per_host(0)
        .proxy(reqwest::Proxy::all(proxy_url(proxy)).map_err(invalid)?)
        .build()
        .map_err(invalid)
}

pub(super) struct Api {
    base: Url,
    client: Client,
    languages: String,
}

impl Api {
    pub(super) fn new(base: Url, client: Client, languages: &[String]) -> Self {
        let languages = languages
            .iter()
            .filter(|tag| tag.is_ascii())
            .cloned()
            .collect::<Vec<_>>()
            .join(",");
        Self {
            base,
            client,
            languages,
        }
    }

    pub(super) async fn catalog(&self, bot: &BotRef) -> Result<ModelCatalog, AgentModelsProblem> {
        let path = agent_models_path(&bot.plugin_id, &bot.bot);
        let catalog: ModelCatalog = self.json(Method::GET, &path, None, false).await?;
        // 核心转之前查过；App 和核心分开升级，网址要在这边再挡一道才交给 App 内的浏览器。
        catalog.check().map_err(unchecked)?;
        Ok(catalog)
    }

    pub(super) async fn models(
        &self,
        bot: &BotRef,
        provider: &str,
    ) -> Result<ModelList, AgentModelsProblem> {
        let path = agent_model_list_path(&bot.plugin_id, &bot.bot, provider);
        let list: ModelList = self.json(Method::GET, &path, None, false).await?;
        list.check().map_err(unchecked)?;
        Ok(list)
    }

    pub(super) async fn save_key(
        &self,
        bot: &BotRef,
        provider: &str,
        key: SecretString,
        base_url: Option<String>,
    ) -> Result<KeyCheck, AgentModelsProblem> {
        let path = agent_model_provider_path(&bot.plugin_id, &bot.bot, provider);
        let body = body(&SaveModelKey { key, base_url });
        self.json(Method::PUT, &path, Some(body), false).await
    }

    pub(super) async fn disconnect(
        &self,
        bot: &BotRef,
        provider: &str,
    ) -> Result<(), AgentModelsProblem> {
        let path = agent_model_provider_path(&bot.plugin_id, &bot.bot, provider);
        self.call(Method::DELETE, &path, None, false)
            .await
            .map(drop)
    }

    pub(super) async fn login_start(
        &self,
        bot: &BotRef,
        provider: &str,
        redirect: bool,
    ) -> Result<ModelLogin, AgentModelsProblem> {
        let path = agent_model_login_start_path(&bot.plugin_id, &bot.bot, provider);
        let body = body(&StartModelLogin {
            redirect_uri: redirect.then(|| MODEL_LOGIN_REDIRECT_URI.to_owned()),
        });
        let login: ModelLogin = self.json(Method::POST, &path, Some(body), false).await?;
        login.checked().map_err(unchecked)
    }

    pub(super) async fn login_poll(
        &self,
        bot: &BotRef,
        session: &str,
    ) -> Result<ModelLoginState, AgentModelsProblem> {
        let path = agent_model_login_path(&bot.plugin_id, &bot.bot, session);
        self.json(Method::GET, &path, None, false).await
    }

    pub(super) async fn login_finish(
        &self,
        bot: &BotRef,
        session: &str,
        callback_url: String,
    ) -> Result<ModelLoginState, AgentModelsProblem> {
        let path = agent_model_login_finish_path(&bot.plugin_id, &bot.bot, session);
        let body = body(&FinishModelLogin { callback_url });
        self.json(Method::POST, &path, Some(body), false).await
    }

    pub(super) async fn login_cancel(
        &self,
        bot: &BotRef,
        session: &str,
    ) -> Result<(), AgentModelsProblem> {
        let path = agent_model_login_path(&bot.plugin_id, &bot.bot, session);
        self.call(Method::DELETE, &path, None, false)
            .await
            .map(drop)
    }

    pub(super) async fn select(
        &self,
        bot: &BotRef,
        select: &SelectModel,
    ) -> Result<ModelSelection, AgentModelsProblem> {
        let path = agent_model_current_path(&bot.plugin_id, &bot.bot);
        self.json(Method::PUT, &path, Some(body(select)), false)
            .await
    }

    pub(super) async fn test(&self, bot: &BotRef) -> Result<ModelTest, AgentModelsProblem> {
        let path = agent_model_test_path(&bot.plugin_id, &bot.bot);
        self.json(Method::POST, &path, None, true).await
    }

    async fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
        test: bool,
    ) -> Result<T, AgentModelsProblem> {
        let bytes = self.call(method, path, body, test).await?;
        serde_json::from_slice(&bytes).map_err(|e| unreadable(e.to_string()))
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
        test: bool,
    ) -> Result<Vec<u8>, AgentModelsProblem> {
        // 协议的路径函数已经逐段编码过（服务商 ID、会话号里的 `:`、`/`），原样放进去，不再编一遍。
        let mut url = self.base.clone();
        url.set_path(path);
        let limit = timeout(test);
        let mut request = self.client.request(method, url).timeout(limit);
        if !self.languages.is_empty() {
            request = request.header(ACCEPT_LANGUAGE, &self.languages);
        }
        if let Some(body) = body {
            request = request.header(CONTENT_TYPE, "application/json").body(body);
        }
        let failed = |e: reqwest::Error| transport_failure(&e, limit);
        let response = request.send().await.map_err(failed)?;
        let status = response.status().as_u16();
        let bytes = response.bytes().await.map_err(failed)?.to_vec();
        if !(200..300).contains(&status) {
            return Err(error_response(status, &bytes));
        }
        Ok(bytes)
    }
}

fn body<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).expect("a model request body always serializes")
}

fn transport_failure(error: &reqwest::Error, limit: Duration) -> AgentModelsProblem {
    let reason = if error.is_timeout() {
        l10n!(
            CATALOG,
            language(),
            "peers-timeout",
            seconds = limit.as_secs()
        )
    } else {
        error.to_string()
    };
    transport(reason)
}

#[derive(Deserialize)]
struct ErrorBody {
    error: CoreFailure,
}

#[derive(Deserialize)]
struct CoreFailure {
    kind: AgentModelsFailure,
    message: String,
    #[serde(default)]
    field: Option<String>,
    #[serde(default)]
    detail: Option<String>,
}

fn reason(kind: AgentModelsFailure) -> AgentModelsReason {
    match kind {
        AgentModelsFailure::Forbidden => AgentModelsReason::Forbidden,
        AgentModelsFailure::NotInstalled => AgentModelsReason::NotInstalled,
        AgentModelsFailure::NotReady => AgentModelsReason::NotReady,
        AgentModelsFailure::Unsupported => AgentModelsReason::Unsupported,
        AgentModelsFailure::NotFound => AgentModelsReason::NotFound,
        AgentModelsFailure::InvalidInput => AgentModelsReason::InvalidInput,
        AgentModelsFailure::Upstream => AgentModelsReason::Upstream,
        AgentModelsFailure::Other => AgentModelsReason::Other,
    }
}

/// 按回应分原因：核心的错误都带 `error.kind`；没有的是本地代理回的（隧道不通、设备被吊销），或核心太旧、还没有这组接口。
pub(super) fn error_response(status: u16, body: &[u8]) -> AgentModelsProblem {
    if let Ok(ErrorBody { error }) = serde_json::from_slice::<ErrorBody>(body) {
        return AgentModelsProblem {
            reason: reason(error.kind),
            message: error.message,
            field: error.field,
            detail: error.detail,
        };
    }
    let message = l10n!(CATALOG, language(), "peers-server-status", status = status);
    match status {
        401 | 403 => problem(
            AgentModelsReason::Forbidden,
            l10n!(CATALOG, language(), "peers-rejected", reason = message),
        ),
        502 | 503 => transport(message),
        _ => problem(AgentModelsReason::Protocol, message),
    }
}
