//! Agent 链列表（ADR-0061 第 10 节）：长轮询 `GET /_wonton/api/agent/chains?after=<版本>`，把还没结束的
//! 链交给壳。和确认列表一样的做法：跟着家里的连接走，连不上时留上次的列表、退避后接着轮询。列表本身没有
//! 出错的显示位（协议的 [`AgentChainsViewModel`] 只有链和版本），取不到的原因只进日志。
//!
//! 动作（停链、改上限、改并发）在 [`actions`]，壳从 `ClientCore` 的单独分派入口发。

use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::watch;
use url::Url;
use wonton_protocol::{
    AGENT_CHAINS_API_PATH, AGENT_CHAINS_WAIT_SECS, AgentChainRow, AgentChainsViewModel,
    HomeViewModel, ProxySettings,
};

use crate::home::HomeLink;

pub(crate) mod actions;

#[cfg(test)]
#[path = "agent_chains_tests.rs"]
mod agent_chains_tests;

/// 长轮询最多挂 [`AGENT_CHAINS_WAIT_SECS`]，客户端的超时要比它长。
const POLL_TIMEOUT: Duration = Duration::from_secs(AGENT_CHAINS_WAIT_SECS + 10);

/// 长轮询失败后的退避：连不上一两次多半是抖动，翻倍着退，到上限为止（和确认列表一致）。
const BACKOFF_START: Duration = Duration::from_secs(2);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// 链列表的跟随者：家里连着就一直长轮询，把最新的一份列表发给壳。
pub(crate) struct AgentChainsLink {
    home: Arc<HomeLink>,
    view: watch::Sender<AgentChainsViewModel>,
}

impl AgentChainsLink {
    pub(crate) fn new(
        handle: Handle,
        home: Arc<HomeLink>,
        view: watch::Sender<AgentChainsViewModel>,
    ) -> Arc<Self> {
        let link = Arc::new(Self {
            home: home.clone(),
            view,
        });
        // 连接的 watch 接收端归这个任务自己：`changed()` 要可变借，任务里独占（同确认列表的跟随者）。
        handle.spawn(follow(link.clone(), home.subscribe()));
        link
    }

    pub(crate) fn sender(&self) -> watch::Sender<AgentChainsViewModel> {
        self.view.clone()
    }
}

async fn follow(link: Arc<AgentChainsLink>, mut updates: watch::Receiver<HomeViewModel>) {
    let mut after: Option<u64> = None;
    let mut backoff = Duration::ZERO;
    loop {
        let Some((core_url, proxy)) = wait_ready(&link, &mut updates, backoff).await else {
            return;
        };
        match fetch_list(&core_url, &proxy, after).await {
            Ok(view) => {
                after = Some(view.revision);
                backoff = Duration::ZERO;
                link.view.send_replace(view);
            }
            Err(failure) => {
                tracing::warn!("the chain list fetch failed: {}", failure.describe());
                backoff = if backoff.is_zero() {
                    BACKOFF_START
                } else {
                    (backoff * 2).min(BACKOFF_MAX)
                };
            }
        }
    }
}

/// 连接没就绪就不打核心；退避没走完先等退避。家里的连接没了（作用域在收尾）返回 `None`。
async fn wait_ready(
    link: &AgentChainsLink,
    updates: &mut watch::Receiver<HomeViewModel>,
    backoff: Duration,
) -> Option<(Url, ProxySettings)> {
    if !backoff.is_zero() {
        tokio::select! {
            () = tokio::time::sleep(backoff) => {}
            changed = updates.changed() => {
                changed.ok()?;
            }
        }
    }
    loop {
        let proxy = link.home.view().proxy;
        if let (Some(proxy), Some(core_url)) = (proxy, link.home.core_url()) {
            return Some((core_url, proxy));
        }
        updates.changed().await.ok()?;
    }
}

/// 列表取不到的原因。列表没有出错的显示位，原因只进日志，退避接着轮询。
pub(super) enum FetchFailure {
    /// 本地代理、隧道连不上，或回应没读完。
    Transport(reqwest::Error),
    /// 核心回了错误状态码。
    Status(u16),
    /// 地址拼不出来，或回应不是这份 ViewModel。
    Malformed(String),
}

impl FetchFailure {
    fn describe(&self) -> String {
        match self {
            Self::Transport(error) => error.to_string(),
            Self::Status(status) => format!("HTTP {status}"),
            Self::Malformed(reason) => reason.clone(),
        }
    }
}

/// `GET <核心地址>/_wonton/api/agent/chains`：带 `after` 时核心没有更新的就挂住。
pub(super) async fn fetch_list(
    core_url: &Url,
    proxy: &ProxySettings,
    after: Option<u64>,
) -> Result<AgentChainsViewModel, FetchFailure> {
    let mut url = core_url
        .join(AGENT_CHAINS_API_PATH)
        .map_err(|error| FetchFailure::Malformed(error.to_string()))?;
    if let Some(after) = after {
        url.query_pairs_mut()
            .append_pair("after", &after.to_string());
    }
    let client = client(proxy, POLL_TIMEOUT).map_err(FetchFailure::Transport)?;
    let (status, body) = exchange(client, reqwest::Method::GET, url, None)
        .await
        .map_err(FetchFailure::Transport)?;
    if !(200..300).contains(&status) {
        return Err(FetchFailure::Status(status));
    }
    serde_json::from_slice(&body).map_err(|error| FetchFailure::Malformed(error.to_string()))
}

/// 停链成功后核心把停后的整行给回来：就地换进列表，壳马上看到停了。版本不动：下一次长轮询带着旧
/// `after` 也会等到新版本，带回整表。
pub(crate) fn apply_row(view: &mut AgentChainsViewModel, row: AgentChainRow) {
    if let Some(slot) = view
        .chains
        .iter_mut()
        .find(|existing| existing.chain == row.chain)
    {
        *slot = row;
    }
}

/// 走本地代理的客户端，核心凭隧道身份认出这台已配对设备。超时管整个请求（含读回应）。
pub(crate) fn client(
    proxy: &ProxySettings,
    timeout: Duration,
) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(crate::chat::proxy_url(proxy))?)
        .timeout(timeout)
        .build()
}

/// 发一个请求、读完回应。请求体按 JSON 发。
pub(crate) async fn exchange(
    client: reqwest::Client,
    method: reqwest::Method,
    url: Url,
    body: Option<Vec<u8>>,
) -> Result<(u16, Vec<u8>), reqwest::Error> {
    let mut request = client.request(method, url);
    if let Some(body) = body {
        request = request
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);
    }
    let response = request.send().await?;
    let status = response.status().as_u16();
    let bytes = response.bytes().await?;
    Ok((status, bytes.to_vec()))
}
