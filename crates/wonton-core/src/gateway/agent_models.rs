//! Agent 插件模型接入的按 Bot 路由（ADR-0129）：认人、核对这个 Bot 归不归他，查过输入再交给插件命令。
//!
//! 只看的操作（目录、模型列表）和改的一样要权限：别人 Bot 接了哪家、账号名是什么也不给看。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use hyper::header::HeaderValue;
use hyper::{HeaderMap, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use wonton_protocol::{
    AgentModelsFailure, BotKeeper, FinishModelLogin, KeyCheck, MAX_MODELS_ID_CHARS,
    MAX_MODELS_TEXT_CHARS, MODEL_LOGIN_REDIRECT_URI, ModelCatalog, ModelList, ModelLogin,
    ModelLoginState, ModelSelection, ModelTest, ModelsOp, PermissionItem, SaveModelKey,
    SecretString, SelectModel, StartModelLogin,
};

use super::api::languages;
use super::{Gateway, Person, TunnelPeer, pages, permissions};
use crate::plugin::agent_models::{AgentModelsError, ModelsOutput, ModelsTarget, failure};
use crate::plugin::languages::Languages;

/// 一把 key 最长多少字节；再长的不是 key。
const MAX_KEY_BYTES: usize = 8 * 1024;

const BASE: &str = "/_wonton/api/bots/{id}/{bot}/models";

pub(super) fn routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route(BASE, get(catalog))
        .route(
            &format!("{BASE}/providers/{{provider}}"),
            put(save_key).delete(disconnect),
        )
        .route(
            &format!("{BASE}/providers/{{provider}}/models"),
            get(models),
        )
        .route(
            &format!("{BASE}/providers/{{provider}}/login"),
            post(login_start),
        )
        .route(
            &format!("{BASE}/logins/{{session}}"),
            get(login_poll).delete(login_cancel),
        )
        .route(
            &format!("{BASE}/logins/{{session}}/finish"),
            post(login_finish),
        )
        .route(&format!("{BASE}/current"), put(select))
        .route(&format!("{BASE}/test"), post(test))
}

fn status(kind: AgentModelsFailure) -> StatusCode {
    match kind {
        AgentModelsFailure::Forbidden => StatusCode::FORBIDDEN,
        AgentModelsFailure::NotInstalled
        | AgentModelsFailure::Unsupported
        | AgentModelsFailure::NotFound => StatusCode::NOT_FOUND,
        AgentModelsFailure::NotReady => StatusCode::SERVICE_UNAVAILABLE,
        AgentModelsFailure::InvalidInput => StatusCode::BAD_REQUEST,
        AgentModelsFailure::Upstream => StatusCode::BAD_GATEWAY,
        AgentModelsFailure::Other => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn error(error: AgentModelsError) -> Response {
    let kind = serde_json::to_value(error.kind)
        .ok()
        .and_then(|kind| kind.as_str().map(str::to_owned))
        .unwrap_or_else(|| "other".to_owned());
    let mut body = json!({"error": {"kind": kind, "message": error.message}});
    if let Some(field) = error.field {
        body["error"]["field"] = field.into();
    }
    if let Some(detail) = error.detail {
        body["error"]["detail"] = detail.into();
    }
    let mut response = (status(error.kind), Json(body)).into_response();
    if let Ok(value) = HeaderValue::from_str(&kind) {
        response.headers_mut().insert(pages::ERROR_HEADER, value);
    }
    response
}

fn invalid(langs: &Languages, field: &str) -> AgentModelsError {
    AgentModelsError {
        field: Some(field.to_owned()),
        ..failure(
            langs,
            AgentModelsFailure::InvalidInput,
            "agent-models-invalid-input",
            None,
        )
    }
}

/// 私人 Bot 归他；共享 Bot 要能管共享 Bot。主人什么都能改。
fn may_change(gateway: &Gateway, person: &Person, keeper: &BotKeeper) -> bool {
    if person.is_owner() {
        return true;
    }
    match keeper {
        BotKeeper::User { username } => *username == person.username,
        BotKeeper::Household => gateway.permissions(person).server_settings,
        BotKeeper::Other => false,
    }
}

/// 认过人、找到了 Bot 的一次请求。
struct Call {
    gateway: Arc<Gateway>,
    target: ModelsTarget,
    langs: Languages,
    who: String,
    plugin: String,
}

impl Call {
    fn begin(
        gateway: Arc<Gateway>,
        peer: Option<Extension<TunnelPeer>>,
        headers: &HeaderMap,
        plugin: String,
        bot: &str,
    ) -> Result<Self, Box<Response>> {
        if let Some(denied) =
            permissions::require(&gateway, peer, headers, PermissionItem::ModelSettings)
        {
            return Err(Box::new(denied));
        }
        let langs = languages(headers);
        // 主人令牌没有设备身份，按主人算。
        let person = gateway.device_person(peer.map(|Extension(peer)| peer));
        let target = gateway
            .plugins
            .agent_models_target(&plugin, bot, &langs)
            .map_err(|failure| Box::new(error(failure)))?;
        if let Some(person) = &person
            && !may_change(&gateway, person, &target.keeper)
        {
            return Err(Box::new(error(failure(
                &langs,
                AgentModelsFailure::Forbidden,
                "agent-models-forbidden",
                None,
            ))));
        }
        let who = person.map_or_else(
            || gateway.owner.username().to_owned(),
            |person| person.username,
        );
        Ok(Self {
            gateway,
            target,
            langs,
            who,
            plugin,
        })
    }

    /// 审计只记谁、哪个 Bot、什么操作、哪个服务商，不记 key、网址和会话。
    fn audit(&self, op: &str, provider: Option<&str>) {
        tracing::info!(
            who = %self.who,
            plugin = %self.plugin,
            bot = %self.target.bot,
            op,
            provider = provider.unwrap_or(""),
            "Agent 模型接入"
        );
    }

    async fn run<T: ModelsOutput + Serialize>(self, op: ModelsOp) -> Response {
        match self
            .gateway
            .plugins
            .run_agent_models::<T>(&self.target, op, &self.langs)
            .await
        {
            Ok(value) => Json(value).into_response(),
            Err(failure) => error(failure),
        }
    }

    async fn run_done(self, op: ModelsOp) -> Response {
        match self
            .gateway
            .plugins
            .exec_agent_models(&self.target, op, &self.langs)
            .await
        {
            Ok(_) => StatusCode::NO_CONTENT.into_response(),
            Err(failure) => error(failure),
        }
    }
}

fn id(langs: &Languages, field: &str, value: &str) -> Result<(), AgentModelsError> {
    if value.trim().is_empty()
        || value.chars().count() > MAX_MODELS_ID_CHARS
        || value.chars().any(char::is_control)
    {
        return Err(invalid(langs, field));
    }
    Ok(())
}

fn body<T: DeserializeOwned>(langs: &Languages, bytes: &Bytes) -> Result<T, AgentModelsError> {
    serde_json::from_slice(bytes).map_err(|_| {
        failure(
            langs,
            AgentModelsFailure::InvalidInput,
            "agent-models-invalid-body",
            None,
        )
    })
}

/// 粘贴来的 key 常带首尾空白和换行，去掉；中间有空白或控制字符的不是 key。
fn key(langs: &Languages, key: &SecretString) -> Result<SecretString, AgentModelsError> {
    let trimmed = key.expose().trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_KEY_BYTES
        || trimmed.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(invalid(langs, "key"));
    }
    Ok(SecretString::new(trimmed))
}

/// 自定义服务商的地址：可以是局域网里的 http，但必须是 http(s) 网址。
fn base_url(langs: &Languages, value: Option<String>) -> Result<Option<String>, AgentModelsError> {
    let Some(value) = value
        .map(|value| value.trim().to_owned())
        .filter(|v| !v.is_empty())
    else {
        return Ok(None);
    };
    match url::Url::parse(&value) {
        Ok(url)
            if matches!(url.scheme(), "http" | "https")
                && url.host().is_some()
                && value.chars().count() <= MAX_MODELS_TEXT_CHARS =>
        {
            Ok(Some(value))
        }
        _ => Err(invalid(langs, "base_url")),
    }
}

/// 回跳网址必须回到 App 的模型登录地址，别的网址不交给插件。
fn callback(langs: &Languages, url: &str) -> Result<(), AgentModelsError> {
    let rest = url.strip_prefix(MODEL_LOGIN_REDIRECT_URI);
    if rest.is_some_and(|rest| rest.is_empty() || rest.starts_with(['?', '#', '/']))
        && url.len() <= 8 * 1024
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Ok(());
    }
    Err(invalid(langs, "callback_url"))
}

type BotPath = Path<(String, String)>;
type ItemPath = Path<(String, String, String)>;

macro_rules! begin {
    ($gateway:expr, $peer:expr, $headers:expr, $plugin:expr, $bot:expr) => {
        match Call::begin($gateway, $peer, &$headers, $plugin, &$bot) {
            Ok(call) => call,
            Err(response) => return *response,
        }
    };
}

macro_rules! check {
    ($value:expr) => {
        match $value {
            Ok(value) => value,
            Err(failure) => return error(failure),
        }
    };
}

async fn catalog(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin, bot)): BotPath,
) -> Response {
    let call = begin!(gateway, peer, headers, plugin, bot);
    call.audit("catalog", None);
    call.run::<ModelCatalog>(ModelsOp::Catalog).await
}

async fn save_key(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin, bot, provider)): ItemPath,
    bytes: Bytes,
) -> Response {
    let call = begin!(gateway, peer, headers, plugin, bot);
    let langs = &call.langs;
    check!(id(langs, "provider", &provider));
    let request: SaveModelKey = check!(body(langs, &bytes));
    let key = check!(key(langs, &request.key));
    let base_url = check!(base_url(langs, request.base_url));
    call.audit("save_key", Some(&provider));
    call.run::<KeyCheck>(ModelsOp::SaveKey {
        provider,
        key,
        base_url,
    })
    .await
}

async fn disconnect(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin, bot, provider)): ItemPath,
) -> Response {
    let call = begin!(gateway, peer, headers, plugin, bot);
    check!(id(&call.langs, "provider", &provider));
    call.audit("disconnect", Some(&provider));
    call.run_done(ModelsOp::Disconnect { provider }).await
}

async fn models(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin, bot, provider)): ItemPath,
) -> Response {
    let call = begin!(gateway, peer, headers, plugin, bot);
    check!(id(&call.langs, "provider", &provider));
    call.audit("models", Some(&provider));
    call.run::<ModelList>(ModelsOp::Models { provider }).await
}

async fn login_start(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin, bot, provider)): ItemPath,
    bytes: Bytes,
) -> Response {
    let call = begin!(gateway, peer, headers, plugin, bot);
    let langs = &call.langs;
    check!(id(langs, "provider", &provider));
    let request: StartModelLogin = if bytes.is_empty() {
        StartModelLogin { redirect_uri: None }
    } else {
        check!(body(langs, &bytes))
    };
    if request
        .redirect_uri
        .as_deref()
        .is_some_and(|uri| uri != MODEL_LOGIN_REDIRECT_URI)
    {
        return error(invalid(langs, "redirect_uri"));
    }
    call.audit("login_start", Some(&provider));
    call.run::<ModelLogin>(ModelsOp::LoginStart {
        provider,
        redirect_uri: request.redirect_uri,
    })
    .await
}

async fn login_poll(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin, bot, session)): ItemPath,
) -> Response {
    let call = begin!(gateway, peer, headers, plugin, bot);
    check!(id(&call.langs, "session", &session));
    call.run::<ModelLoginState>(ModelsOp::LoginPoll { session })
        .await
}

async fn login_finish(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin, bot, session)): ItemPath,
    bytes: Bytes,
) -> Response {
    let call = begin!(gateway, peer, headers, plugin, bot);
    let langs = &call.langs;
    check!(id(langs, "session", &session));
    let request: FinishModelLogin = check!(body(langs, &bytes));
    check!(callback(langs, &request.callback_url));
    call.audit("login_finish", None);
    call.run::<ModelLoginState>(ModelsOp::LoginFinish {
        session,
        callback_url: request.callback_url,
    })
    .await
}

async fn login_cancel(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin, bot, session)): ItemPath,
) -> Response {
    let call = begin!(gateway, peer, headers, plugin, bot);
    check!(id(&call.langs, "session", &session));
    call.run_done(ModelsOp::LoginCancel { session }).await
}

async fn select(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin, bot)): BotPath,
    bytes: Bytes,
) -> Response {
    let call = begin!(gateway, peer, headers, plugin, bot);
    let langs = &call.langs;
    let request: SelectModel = check!(body(langs, &bytes));
    check!(id(langs, "provider", &request.provider));
    if request.model.trim().is_empty()
        || request.model.chars().count() > MAX_MODELS_TEXT_CHARS
        || request.model.chars().any(char::is_control)
    {
        return error(invalid(langs, "model"));
    }
    call.audit("select", Some(&request.provider));
    call.run::<ModelSelection>(ModelsOp::Select {
        provider: request.provider,
        model: request.model,
        confirmed: request.confirmed,
    })
    .await
}

async fn test(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin, bot)): BotPath,
) -> Response {
    let call = begin!(gateway, peer, headers, plugin, bot);
    call.audit("test", None);
    call.run::<ModelTest>(ModelsOp::Test).await
}

#[cfg(test)]
#[path = "agent_models_tests.rs"]
mod tests;
