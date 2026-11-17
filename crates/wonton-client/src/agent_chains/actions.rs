//! 链的动作：停链、改上限、改使用方并发（ADR-0061 第 10 节）。请求走本地代理，核心凭隧道身份认出这台
//! 已配对设备；动作不重试，异步失败按 [`CoreError`] 的类别发布到链列表。

use std::time::Duration;

use reqwest::Method;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{
    AGENT_LIMITS_PATH, AGENT_SETTINGS_PATH, AgentAction, AgentChainRow, AgentConcurrency,
    AgentSettingsViewModel, CoreError, ProxySettings, agent_chain_stop_path,
    agent_consumer_concurrency_path,
};

use crate::l10n::{CATALOG, language};

use super::{client, exchange};

/// 动作的整个请求最多等这么久：停链、改上限、改并发都是核心马上回的快操作。
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// 一个动作做完的局部效果：停链成功时核心把停后的整行给回来，就地换进列表；其余动作核心只确认。
#[derive(Debug)]
pub(crate) enum Effect {
    Stopped(AgentChainRow),
    Done,
}

/// 执行一个链动作。网络和回应的错误都带类别；回应体是核心给的类型，解析不出来按上游出错报。
pub(crate) async fn perform(
    core_url: &Url,
    proxy: &ProxySettings,
    action: &AgentAction,
) -> Result<Effect, CoreError> {
    let client = client(proxy, ACTION_TIMEOUT).map_err(|error| client_error(&error.to_string()))?;
    match action {
        AgentAction::StopChain { chain } => {
            let url = url(core_url, &agent_chain_stop_path(chain));
            let (status, body) = send(&client, Method::POST, url, None).await?;
            Ok(Effect::Stopped(parse(status, &body)?))
        }
        AgentAction::SetAgentLimits { limits } => {
            let url = url(core_url, AGENT_LIMITS_PATH);
            let body = serde_json::to_vec(limits).expect("the limits always serialize to JSON");
            let (status, response) = send(&client, Method::PUT, url, Some(body)).await?;
            let _: AgentSettingsViewModel = parse(status, &response)?;
            Ok(Effect::Done)
        }
        AgentAction::SetConsumerAgentConcurrency {
            plugin_id,
            concurrency,
        } => {
            let url = url(core_url, &agent_consumer_concurrency_path(plugin_id));
            let body = serde_json::to_vec(&AgentConcurrency {
                concurrency: *concurrency,
            })
            .expect("the concurrency always serializes to JSON");
            let (status, response) = send(&client, Method::PUT, url, Some(body)).await?;
            let _: AgentConcurrency = parse(status, &response)?;
            Ok(Effect::Done)
        }
    }
}

/// `GET /_wonton/api/agent/settings`：设置“Agent”一页的当前值。改上限前要有当前值可读，核心接口带着。
pub(crate) async fn settings(
    core_url: &Url,
    proxy: &ProxySettings,
) -> Result<AgentSettingsViewModel, CoreError> {
    let client = client(proxy, ACTION_TIMEOUT).map_err(|error| client_error(&error.to_string()))?;
    let url = url(core_url, AGENT_SETTINGS_PATH);
    let (status, body) = send(&client, Method::GET, url, None).await?;
    parse(status, &body)
}

/// 路径拼进核心地址。路径来自协议常量和核心发的 ID，核心地址是 http(s)，拼不出来只在地址不是
/// http(s) 的时候（同插件接口的做法）。
fn url(core_url: &Url, path: &str) -> Url {
    core_url
        .join(path)
        .expect("home server URL is http and the path comes from the protocol constants")
}

async fn send(
    client: &reqwest::Client,
    method: Method,
    url: Url,
    body: Option<Vec<u8>>,
) -> Result<(u16, Vec<u8>), CoreError> {
    exchange(client.clone(), method, url, body)
        .await
        .map_err(|error| {
            tracing::info!("the chain action request failed: {error}");
            transport()
        })
}

fn transport() -> CoreError {
    CoreError::TransportUnavailable {
        reason: l10n!(CATALOG, language(), "agent-api-not-connected"),
    }
}

fn client_error(reason: &str) -> CoreError {
    CoreError::Upstream {
        reason: l10n!(CATALOG, language(), "agent-api-client", reason = reason),
    }
}

/// 核心的错误响应：`{"error": {"kind": …, "message": …}}`，和登录、插件接口同一份形状。
#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: String,
}

/// 按状态码分回类别：401 设备不认识；403 身份对但不让做（链不归你、只有主人能改）；404/400 引用了
/// 不存在的对象或输入不合法；502 本地代理到不了隧道；其余按上游出错。核心带了一句话就用它（核心按
/// App 的语言写），没带就按状态码给一句。
fn action_error(status: u16, body: &[u8]) -> CoreError {
    let detail = serde_json::from_slice::<ErrorBody>(body)
        .ok()
        .map(|body| body.error.message);
    let reason =
        detail.unwrap_or_else(|| l10n!(CATALOG, language(), "agent-api-status", status = status));
    match status {
        401 => CoreError::Unauthorized { reason },
        403 => CoreError::Forbidden { reason },
        400 | 404 => CoreError::InvalidInput { reason },
        502 => CoreError::TransportUnavailable { reason },
        _ => CoreError::Upstream { reason },
    }
}

/// 2xx 的回应体必须是动作的回应类型；状态码先按类别报，解析不出来按上游出错。
fn parse<T: DeserializeOwned>(status: u16, body: &[u8]) -> Result<T, CoreError> {
    if status >= 400 {
        return Err(action_error(status, body));
    }
    serde_json::from_slice(body).map_err(|error| CoreError::Upstream {
        reason: l10n!(
            CATALOG,
            language(),
            "agent-api-unreadable",
            error = error.to_string()
        ),
    })
}
