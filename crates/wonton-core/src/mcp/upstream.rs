//! 总线连插件的 MCP 服务：每个插件一个客户端（Streamable HTTP，经插件网络），用到时连上，传输层出错时丢掉，下次重连。核心读文件
//! 和 Bot 的调用共用这条连接，所以插件正常回的错误、一次调用超时都不丢。
//!
//! 只有一种超时会换连接：连续 [`MAX_TIMEOUTS`] 次带期限的请求没等到回答，这之间也没有任何请求拿到过回答，就当这个会话卡死了（比如
//! supergateway 有状态模式下的 stdio 子进程）：TCP 没断，别的办法发现不了。换掉时先取消旧会话，上面还在跑的调用就结束了。
//!
//! 要避开两种误判，否则慢但还活着的会话被换掉，有状态的 stdio 子进程跟着丢：
//! - 同时超时的算一次：几个 Agent 在同一次慢的时候一起超时，不是“连续”。只计“上一次回答或者上一次计数之后才开始的请求”。
//! - 串行的服务器（stdio）一次只做一件事，长调用会让 `tools/list` 排在后面。所以有更早开始的 `tools/call` 还在跑时，超时不算：它
//!   本身有期限（[`crate::plugin::ToolLimits`]），到期后要是还不回答，再计数。
//!
//! `tools/call` 带进度令牌（rmcp 总是带），收到 `notifications/progress` 就重新计空闲期限，另有总期限。到期后发
//! `notifications/cancelled`，这时不知道插件做到了哪一步，结果算不确定（[`CallError::Uncertain`]）。

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, CancelledNotificationParam,
    ClientCapabilities, ClientConfig, ClientRequest, Implementation, JsonObject,
    PaginatedRequestParams, ProtocolVersion, ReadResourceRequestParams, ResourceContents,
    ServerResult, Tool,
};
use rmcp::service::{PeerRequestOptions, RoleClient, RunningService, ServiceError};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransportConfig, StreamableHttpError,
};
use rmcp::{ClientLifecycleMode, ClientServiceExt};
use serde_json::Value;
use wonton_protocol::{TOOL_CALL_CAPABILITIES_META, TOOL_CALL_USER_META};

use tokio::time::Instant;

use super::bounded_http::{
    BoundedHttp, HttpError, MAX_TOOLS_BYTES, RESPONSE_TOO_LARGE, TooLargeDetail, too_large_detail,
};
use super::file_results::human_size;
use crate::plugin::{McpHeaders, McpServer, PluginId};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// 插件接受了连接却不回握手：不能一直等。
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// 读一个资源最多这么久。
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// 列一个插件的工具（包括没连上时的握手）最多这么久。
const TOOLS_TIMEOUT: Duration = Duration::from_secs(15);
/// 见模块说明。
const MAX_TIMEOUTS: u32 = 3;
/// 插件的一个响应最多这么多字节：`resources/read` 的 32 MiB 经 base64 变成 43 MiB，再加 JSON。
const MAX_MESSAGE_BYTES: usize = 48 << 20;
/// 一个插件的工具列表最多翻这么多页、这么多个工具，总共 [`MAX_TOOLS_BYTES`]：rmcp 的 `list_all_tools` 一直跟着 `next_cursor` 翻，
/// 而每个 Agent 都会并发地列所有插件，一个插件就能让核心用完内存。
const MAX_TOOL_PAGES: usize = 50;
const MAX_TOOLS: usize = 2_000;
/// 读一个资源最长要多久：连接（包括握手）加读取。调用方的期限要比它长，这里的超时才记得上（[`Upstreams::timed_out`]）。
pub const READ_BUDGET: Duration = CONNECT_TIMEOUT
    .saturating_add(HANDSHAKE_TIMEOUT)
    .saturating_add(READ_TIMEOUT);

/// [`Upstreams::read_resource`] 读不到的原因。
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("{0}")]
    Unreachable(String),
    #[error("{0}")]
    Upstream(String),
    #[error("资源超过上限")]
    TooLarge,
}

/// [`Upstreams::call`] 没拿到结果的原因。
#[derive(Debug, thiserror::Error)]
pub enum CallError {
    /// 没发出去：插件连不上。
    #[error("{0}")]
    NotSent(String),
    /// 插件回了错误。
    #[error("{0}")]
    Refused(String),
    /// 发出去了，没等到结果：可能已经执行了。
    #[error("{message}")]
    Uncertain {
        kind: UncertainKind,
        message: String,
    },
    /// 调用方自己取消了。
    #[error("调用方取消了这次调用")]
    Cancelled,
}

/// 结果不确定的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UncertainKind {
    /// 到期了，已经告诉插件取消。
    TimedOut,
    /// 这条会话不回应，已被换掉。
    SessionReplaced,
    /// 连接断了。
    ConnectionLost,
    /// 插件回了结果，超过上限读不下。
    ResultTooLarge,
}

impl UncertainKind {
    /// 工具结果 `_meta` 里给 Agent 看的类别（插件规范的“工具调用的期限”）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TimedOut => "tool_timed_out",
            Self::SessionReplaced => "session_replaced",
            Self::ConnectionLost => "connection_lost",
            Self::ResultTooLarge => "result_too_large",
        }
    }
}

type Service = RunningService<RoleClient, ClientConfig>;

/// 一条连接。正在跑的调用各持有一份：连接从表里丢掉时它们还能跑完，除非这条会话被换掉（见模块说明）。
struct Connection {
    service: Service,
    /// 卡死被换掉了：这之后结束的调用不是连接断了。
    replaced: AtomicBool,
    /// 正在跑的 `tools/call`、`resources/read` 什么时候开始的。
    calls: Mutex<Vec<Instant>>,
}

impl Connection {
    fn new(service: Service) -> Self {
        Self {
            service,
            replaced: AtomicBool::new(false),
            calls: Mutex::default(),
        }
    }

    fn peer(&self) -> &rmcp::service::Peer<RoleClient> {
        self.service.peer()
    }

    fn calling(&self, started: Instant) -> Calling<'_> {
        self.calls.lock().expect("调用表锁中毒").push(started);
        Calling {
            connection: self,
            started,
        }
    }

    /// 有比 `started` 更早开始、还在跑的调用。
    fn busy_since_before(&self, started: Instant) -> bool {
        let calls = self.calls.lock().expect("调用表锁中毒");
        calls.iter().any(|call| *call < started)
    }
}

struct Calling<'a> {
    connection: &'a Connection,
    started: Instant,
}

impl Drop for Calling<'_> {
    fn drop(&mut self) {
        let mut calls = self.connection.calls.lock().expect("调用表锁中毒");
        if let Some(at) = calls.iter().position(|call| *call == self.started) {
            calls.swap_remove(at);
        }
    }
}

/// 一个插件的连接。
struct Slot {
    url: String,
    /// 建这条连接时用的请求头：密钥换了头就变，连接要跟着换。
    headers: McpHeaders,
    /// 握手只持有这一个插件的锁：卡住的插件挡不住别的插件。
    handshake: Arc<tokio::sync::Mutex<()>>,
    current: Option<Arc<Connection>>,
    /// `current` 上连续超时的次数。
    timeouts: u32,
    /// 上一次回答或者上一次计数的时候：这之前开始的请求超时不算，见模块说明。
    counted_until: Instant,
    /// 最近一次列工具失败的原因，给主人和 Agent 看：工具不见了不能只有核心日志知道。
    tools_problem: Option<String>,
}

impl Slot {
    fn new(url: String, headers: McpHeaders) -> Self {
        Self {
            url,
            headers,
            handshake: Arc::default(),
            current: None,
            timeouts: 0,
            counted_until: Instant::now(),
            tools_problem: None,
        }
    }
}

/// 一次调用失败的原因。
enum CallFailure {
    /// 这个响应超过上限。连接本身没事，带上传输层知道的多少（bounded_http 写在错误 `data` 里）。
    TooLarge(TooLargeDetail),
    /// 这条会话不回应，已被换掉。
    Replaced,
    /// 连接出了问题，已经丢掉。
    Transport(ServiceError),
    /// 插件回了错误，或者回答不合协议。连接本身没事。
    Plugin(ServiceError),
}
/// 转给插件时随调用带上的调用者身份（ADR-0088 第 8 节）。
pub(crate) struct CallIdentity {
    /// 替谁办事：`_meta` 的 `app.wonton/user`。
    pub(crate) user: Option<String>,
    /// 这个人在这个应用里的能力：`_meta` 的 `app.wonton/capabilities`。
    pub(crate) capabilities: Vec<String>,
}

impl CallIdentity {
    /// 认不出调用者：插件只能当全家共用的数据。
    pub(crate) const EMPTY: Self = Self {
        user: None,
        capabilities: Vec::new(),
    };
}

pub struct Upstreams {
    http: reqwest::Client,
    message_limit: usize,
    handshake_timeout: Duration,
    read_timeout: Duration,
    tools_timeout: Duration,
    clients: Mutex<HashMap<PluginId, Slot>>,
}

impl Default for Upstreams {
    fn default() -> Self {
        Self::new()
    }
}

impl Upstreams {
    pub fn new() -> Self {
        // 插件的 MCP 服务在插件网络里，只走 http：不走代理，不跟随跳转，也不加载系统的根证书（用不上，镜像里没有
        // 时构造客户端还会失败）。
        let http = reqwest::Client::builder()
            .no_proxy()
            .tls_certs_only(std::iter::empty())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            // 不留空闲连接：插件停了以后，往池子里那条还开着的连接上发的请求分不清发没发出，
            // 会把“没发出”报成“不确定”。不复用就一定是新建连接，停了就连不上（NotSent）。
            .pool_max_idle_per_host(0)
            .build()
            .expect("构造 MCP 客户端的 HTTP 客户端");
        Self {
            http,
            message_limit: MAX_MESSAGE_BYTES,
            handshake_timeout: HANDSHAKE_TIMEOUT,
            read_timeout: READ_TIMEOUT,
            tools_timeout: TOOLS_TIMEOUT,
            clients: Mutex::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_limits(message_limit: usize, handshake_timeout: Duration) -> Self {
        Self {
            message_limit,
            handshake_timeout,
            ..Self::new()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_read_timeout(read_timeout: Duration) -> Self {
        Self {
            read_timeout,
            ..Self::new()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_tools_timeout(tools_timeout: Duration) -> Self {
        Self {
            tools_timeout,
            ..Self::new()
        }
    }

    /// 测试先用宽松的期限连上，再缩短：期限短到连握手也包进去的话，CI 负载一高就失败。
    #[cfg(test)]
    pub(crate) fn set_tools_timeout(&mut self, tools_timeout: Duration) {
        self.tools_timeout = tools_timeout;
    }

    /// 测试用的不带身份调用。
    #[cfg(test)]
    pub(crate) async fn call(
        &self,
        server: &McpServer,
        tool: &str,
        arguments: Option<JsonObject>,
        cancelled: impl Future<Output = ()>,
    ) -> Result<CallToolResult, CallError> {
        self.call_with_identity(server, tool, arguments, CallIdentity::EMPTY, cancelled)
            .await
    }

    /// 还能用的连接；没有时给出握手要持有的锁。地址或者请求头（密钥换过了）和现在的不一样就换新槽。
    fn live(&self, server: &McpServer) -> Result<Arc<Connection>, Arc<tokio::sync::Mutex<()>>> {
        let mut clients = self.clients.lock().expect("MCP 客户端表锁中毒");
        let slot = clients
            .entry(server.id.clone())
            .or_insert_with(|| Slot::new(server.url.clone(), server.headers.clone()));
        if slot.url != server.url || slot.headers != server.headers {
            *slot = Slot::new(server.url.clone(), server.headers.clone());
        }
        match &slot.current {
            Some(connection) if !connection.service.is_closed() => Ok(connection.clone()),
            _ => Err(slot.handshake.clone()),
        }
    }

    async fn connection(&self, server: &McpServer) -> Result<Arc<Connection>, String> {
        let handshake = match self.live(server) {
            Ok(connection) => return Ok(connection),
            Err(handshake) => handshake,
        };
        let _handshaking = handshake.lock().await;
        // 等锁的时候别的调用可能已经连上了。
        if let Ok(connection) = self.live(server) {
            return Ok(connection);
        }
        let http = match &server.socket {
            Some(path) => reqwest::Client::builder()
                .unix_socket(path.clone())
                .no_proxy()
                // 同 `Upstreams::new()`：会话跨调用缓存，留着空闲连接，插件停了以后的调用会被报成“不确定”。
                .pool_max_idle_per_host(0)
                .build()
                .map_err(|e| format!("连不上插件 {} 的 MCP 套接字：{e}", server.id))?,
            None => self.http.clone(),
        };
        // 清单 `mcp.headers` 的头（密钥已解析）加在发往上游的每个请求上，包括 initialize 握手。
        let mut config = StreamableHttpClientTransportConfig::with_uri(server.url.clone());
        config.custom_headers = server.headers.custom_headers();
        let transport = StreamableHttpClientTransport::with_client(
            BoundedHttp {
                http,
                limit: self.message_limit,
            },
            config,
        );
        // 用 initialize 握手（2025-11-25），不用 2026-07-28 的 server/discover：插件的 MCP 服务大多还不支持新协议，
        // supergateway 4.2.1 转 stdio 时收到 server/discover 就断开、不回应（实测），客户端会一直等下去。
        let info = ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("wonton-core", env!("CARGO_PKG_VERSION")),
        )
        .with_protocol_version(ProtocolVersion::V_2025_11_25);
        let lifecycle = ClientLifecycleMode::Initialize;
        let service = tokio::time::timeout(
            self.handshake_timeout,
            info.serve_with_lifecycle(transport, lifecycle),
        )
        .await
        .map_err(|_| {
            format!(
                "连不上插件 {} 的 MCP 服务：{:?} 内没有完成握手",
                server.id, self.handshake_timeout
            )
        })?
        .map_err(|e| format!("连不上插件 {} 的 MCP 服务：{e}", server.id))?;
        let connection = Arc::new(Connection::new(service));
        let mut clients = self.clients.lock().expect("MCP 客户端表锁中毒");
        // 握手期间地址变了或者插件卸了：这条连接只给这次调用用。
        if let Some(slot) = clients.get_mut(&server.id)
            && Arc::ptr_eq(&slot.handshake, &handshake)
        {
            slot.current = Some(connection.clone());
            slot.timeouts = 0;
            slot.counted_until = Instant::now();
        }
        Ok(connection)
    }

    /// 只丢出错的那条连接：别的调用可能已经连上了新的。
    fn forget(&self, id: &PluginId, broken: &Arc<Connection>) {
        let mut clients = self.clients.lock().expect("MCP 客户端表锁中毒");
        if let Some(slot) = clients.get_mut(id)
            && slot
                .current
                .as_ref()
                .is_some_and(|s| Arc::ptr_eq(s, broken))
        {
            slot.current = None;
        }
    }

    /// 这条连接上有请求拿到了回答（包括插件回的错误）。
    fn answered(&self, id: &PluginId, used: &Arc<Connection>) {
        let mut clients = self.clients.lock().expect("MCP 客户端表锁中毒");
        if let Some(slot) = clients.get_mut(id)
            && slot.current.as_ref().is_some_and(|c| Arc::ptr_eq(c, used))
        {
            slot.timeouts = 0;
            slot.counted_until = Instant::now();
        }
    }

    /// 这条连接上有一个 `started` 开始的请求没等到回答：够多次就换掉，见模块说明。
    fn timed_out(&self, id: &PluginId, used: &Arc<Connection>, started: Instant) {
        let mut clients = self.clients.lock().expect("MCP 客户端表锁中毒");
        let Some(slot) = clients.get_mut(id) else {
            return;
        };
        let current = slot.current.as_ref().is_some_and(|c| Arc::ptr_eq(c, used));
        if !current || started < slot.counted_until || used.busy_since_before(started) {
            return;
        }
        slot.timeouts += 1;
        slot.counted_until = Instant::now();
        if slot.timeouts >= MAX_TIMEOUTS {
            tracing::warn!(plugin = %id, "插件的 MCP 会话连续不回答，换一个新会话");
            used.replaced.store(true, Ordering::SeqCst);
            used.service.cancellation_token().cancel();
            slot.current = None;
            slot.timeouts = 0;
        }
    }

    fn failure(&self, id: &PluginId, used: &Arc<Connection>, error: ServiceError) -> CallFailure {
        match error {
            ServiceError::TransportSend(_) | ServiceError::TransportClosed => {
                if used.replaced.load(Ordering::SeqCst) {
                    return CallFailure::Replaced;
                }
                self.forget(id, used);
                CallFailure::Transport(error)
            }
            ServiceError::McpError(data) if data.code == RESPONSE_TOO_LARGE => {
                self.answered(id, used);
                CallFailure::TooLarge(too_large_detail(data.data.as_ref()))
            }
            other => {
                self.answered(id, used);
                CallFailure::Plugin(other)
            }
        }
    }

    /// 插件现在提供的工具。卡住的插件最多让调用方等 [`TOOLS_TIMEOUT`]（包括没连上时的握手）。
    pub async fn tools(&self, server: &McpServer) -> Result<Vec<Tool>, String> {
        let (listed, problem) = allowlist(server, self.list_tools(server).await);
        let mut clients = self.clients.lock().expect("MCP 客户端表锁中毒");
        if let Some(slot) = clients.get_mut(&server.id) {
            slot.tools_problem = problem.or(listed.as_ref().err().cloned());
        }
        listed
    }

    /// 最近一次列工具为什么失败。
    pub fn tools_problem(&self, id: &PluginId) -> Option<String> {
        let clients = self.clients.lock().expect("MCP 客户端表锁中毒");
        clients.get(id)?.tools_problem.clone()
    }

    /// 不建连接，直接记下列工具的问题（这次没去连的插件用它，见总线列工具时对未配置插件的处理）。
    pub fn note_problem(&self, server: &McpServer, problem: String) {
        let mut clients = self.clients.lock().expect("MCP 客户端表锁中毒");
        clients
            .entry(server.id.clone())
            .or_insert_with(|| Slot::new(server.url.clone(), server.headers.clone()))
            .tools_problem = Some(problem);
    }

    async fn list_tools(&self, server: &McpServer) -> Result<Vec<Tool>, String> {
        let id = &server.id;
        let started = Instant::now();
        let deadline = started + self.tools_timeout;
        let too_slow = || format!("插件 {id} 在 {:?} 内没有列出工具", self.tools_timeout);
        let connection = tokio::time::timeout_at(deadline, self.connection(server))
            .await
            .map_err(|_| too_slow())??;
        let listed = tokio::time::timeout_at(deadline, list_pages(&connection, id)).await;
        match listed {
            Ok(Ok(tools)) => {
                self.answered(id, &connection);
                Ok(tools)
            }
            Ok(Err(ListFailure::Service(e))) => Err(match self.failure(id, &connection, e) {
                CallFailure::TooLarge(_) => format!("插件 {id} 的工具列表超过上限"),
                CallFailure::Replaced => format!("插件 {id} 的会话不回应，已被换掉"),
                CallFailure::Transport(e) | CallFailure::Plugin(e) => {
                    format!("插件 {id} 的 MCP 服务列不出工具：{e}")
                }
            }),
            Ok(Err(ListFailure::Plugin(why))) => {
                self.answered(id, &connection);
                Err(why)
            }
            Err(_) => {
                self.timed_out(id, &connection, started);
                Err(too_slow())
            }
        }
    }

    /// 调用插件的工具，期限见模块说明。带 [`CallIdentity`] 时 `_meta` 同时带用户名和能力，
    /// 认不出人（`user: None`）时两个都不带。
    pub(crate) async fn call_with_identity(
        &self,
        server: &McpServer,
        tool: &str,
        arguments: Option<JsonObject>,
        identity: CallIdentity,
        cancelled: impl Future<Output = ()>,
    ) -> Result<CallToolResult, CallError> {
        let connection = self.connection(server).await.map_err(CallError::NotSent)?;
        let mut params = CallToolRequestParams::new(tool.to_owned());
        if let Some(arguments) = arguments {
            params = params.with_arguments(arguments);
        }
        // 带人时 `_meta` 同时带用户名和能力（ADR-0088 第 8 节）；认不出人就整个不带。
        if let Some(user) = identity.user {
            let mut metadata = serde_json::Map::new();
            metadata.insert(TOOL_CALL_USER_META.to_owned(), Value::from(user));
            metadata.insert(
                TOOL_CALL_CAPABILITIES_META.to_owned(),
                Value::from(identity.capabilities),
            );
            params.meta = Some(rmcp::model::RequestMetaObject(rmcp::model::MetaObject(
                metadata,
            )));
        }
        let limits = server.tool_limits;
        let options = PeerRequestOptions::with_timeout(limits.idle)
            .reset_timeout_on_progress()
            .with_max_total_timeout(limits.total);
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        let started = Instant::now();
        let running = connection.calling(started);
        let id = &server.id;
        let failed = |e: ServiceError| self.call_failure(id, tool, &connection, e);
        // 这里失败时请求还没交给连接，插件没收到。
        let handle = connection
            .peer()
            .send_request_with_option(request, options)
            .await
            .map_err(|e| self.not_sent(id, tool, &connection, e))?;
        let (request_id, peer) = (handle.id.clone(), handle.peer.clone());
        let mut answer = Box::pin(handle.await_response());
        tokio::pin!(cancelled);
        let answer = tokio::select! {
            answer = &mut answer => answer,
            () = &mut cancelled => {
                let reason = Some("调用方取消了".to_owned());
                let param = CancelledNotificationParam::new(Some(request_id), reason);
                if let Err(error) = peer.notify_cancelled(param).await {
                    tracing::warn!(plugin = %id, tool, %error, "取消没能发给插件");
                }
                // 不用 `RequestHandle::cancel`：它和 `await_response` 都要拿走句柄。rmcp 发完取消通知就让这个请求结束，但要
                // `await_response` 跑完才清掉进度表，丢掉它这张表就跟着连接一直涨；取消没发出去时它也会在期限内结束。
                drop(tokio::spawn(answer));
                return Err(CallError::Cancelled);
            }
        };
        drop(running);
        match answer {
            Ok(ServerResult::CallToolResult(result)) => {
                self.answered(id, &connection);
                Ok(result)
            }
            Ok(_) => Err(failed(ServiceError::UnexpectedResponse)),
            Err(ServiceError::Timeout { timeout }) => {
                self.timed_out(id, &connection, started);
                Err(CallError::Uncertain {
                    kind: UncertainKind::TimedOut,
                    message: format!(
                        "插件 {id} 的工具 {tool} 在 {} 秒内没有结果，是否已执行不确定",
                        timeout.as_secs()
                    ),
                })
            }
            Err(e) => Err(failed(e)),
        }
    }

    fn not_sent(
        &self,
        id: &PluginId,
        tool: &str,
        used: &Arc<Connection>,
        error: ServiceError,
    ) -> CallError {
        let why = error.to_string();
        self.failure(id, used, error);
        CallError::NotSent(format!("工具 {tool} 没有发给插件 {id}：{why}"))
    }

    fn call_failure(
        &self,
        id: &PluginId,
        tool: &str,
        used: &Arc<Connection>,
        error: ServiceError,
    ) -> CallError {
        if never_connected(&error) {
            return self.not_sent(id, tool, used, error);
        }
        let uncertain = |kind, message| CallError::Uncertain { kind, message };
        match self.failure(id, used, error) {
            // 上限以传输层执行到的为准（tools/list 的更小）；传输层不知道读到了多少时（插件自己回这个码）不编。
            CallFailure::TooLarge(detail) => {
                let limit = detail.limit.unwrap_or(self.message_limit as u64);
                let how_much = match (detail.declared, detail.read) {
                    (Some(declared), _) => format!("，插件声明有 {}", human_size(declared)),
                    (_, Some(read)) => format!("，至少读了 {}", human_size(read)),
                    (None, None) => String::new(),
                };
                uncertain(
                    UncertainKind::ResultTooLarge,
                    format!(
                        "插件 {id} 的工具 {tool} 的结果超过上限 {}{how_much}，读不到它做了什么",
                        human_size(limit)
                    ),
                )
            }
            CallFailure::Replaced => uncertain(
                UncertainKind::SessionReplaced,
                format!("插件 {id} 的会话不回应，已被换掉，工具 {tool} 是否已执行不确定"),
            ),
            CallFailure::Transport(e) => uncertain(
                UncertainKind::ConnectionLost,
                format!("调用插件 {id} 的工具 {tool} 时连接断了，是否已执行不确定：{e}"),
            ),
            CallFailure::Plugin(e) => {
                CallError::Refused(format!("调用插件 {id} 的工具 {tool} 失败：{e}"))
            }
        }
    }

    /// 读插件的资源（`resources/read`），不超过 `limit` 字节。结果在内存里，也经过一次 JSON：只适合小文件，大文件插件要给内网
    /// 地址。
    pub async fn read_resource(
        &self,
        server: &McpServer,
        uri: &str,
        limit: usize,
    ) -> Result<Vec<u8>, ReadError> {
        let result = self.read_contents(server, uri).await?;
        let Some(contents) = result.contents.into_iter().next() else {
            return Err(ReadError::Upstream(format!(
                "插件 {} 的资源 {uri} 是空的",
                server.id
            )));
        };
        let bytes = match contents {
            ResourceContents::TextResourceContents { text, .. } => text.into_bytes(),
            ResourceContents::BlobResourceContents { blob, .. } => {
                STANDARD.decode(blob).map_err(|e| {
                    ReadError::Upstream(format!(
                        "插件 {} 的资源 {uri} 不是合法的 base64：{e}",
                        server.id
                    ))
                })?
            }
            other => {
                return Err(ReadError::Upstream(format!(
                    "插件 {} 的资源 {uri} 是读不懂的类型：{other:?}",
                    server.id
                )));
            }
        };
        if bytes.len() > limit {
            return Err(ReadError::TooLarge);
        }
        Ok(bytes)
    }

    /// 读插件的资源，连同内容的 `_meta`（MCP Apps 界面要按它设 CSP）。大小上限同上。
    pub async fn read_contents(
        &self,
        server: &McpServer,
        uri: &str,
    ) -> Result<rmcp::model::ReadResourceResult, ReadError> {
        let connection = self
            .connection(server)
            .await
            .map_err(ReadError::Unreachable)?;
        let id = &server.id;
        let params = ReadResourceRequestParams::new(uri.to_owned());
        let started = Instant::now();
        let _running = connection.calling(started);
        let read = connection.peer().read_resource(params);
        let result = match tokio::time::timeout(self.read_timeout, read).await {
            Ok(Ok(result)) => {
                self.answered(id, &connection);
                result
            }
            Ok(Err(e)) => {
                return Err(match self.failure(id, &connection, e) {
                    CallFailure::TooLarge(_) => ReadError::TooLarge,
                    CallFailure::Replaced => {
                        ReadError::Unreachable(format!("插件 {id} 的会话不回应，已被换掉"))
                    }
                    CallFailure::Transport(e) => {
                        ReadError::Unreachable(format!("插件 {id} 读不了资源 {uri}：{e}"))
                    }
                    CallFailure::Plugin(e) => {
                        ReadError::Upstream(format!("插件 {id} 读不了资源 {uri}：{e}"))
                    }
                });
            }
            Err(_) => {
                self.timed_out(id, &connection, started);
                return Err(ReadError::Unreachable(format!(
                    "插件 {id} 的资源 {uri} 在 {:?} 内没有读完",
                    self.read_timeout
                )));
            }
        };
        Ok(result)
    }

    /// 只留下还在运行的插件、地址和请求头都没变的客户端：头变了（密钥换了）就丢掉，下次重连。
    pub fn retain(&self, servers: &[McpServer]) {
        let mut clients = self.clients.lock().expect("MCP 客户端表锁中毒");
        clients.retain(|id, slot| {
            servers
                .iter()
                .any(|s| &s.id == id && s.url == slot.url && s.headers == slot.headers)
        });
    }
}

/// POST 连接都没建起来（`ECONNREFUSED`，插件重启了而缓存的连接还没发现）：插件没收到这个请求。写到一半断了的不算，那可能已经执行了。
fn never_connected(error: &ServiceError) -> bool {
    let ServiceError::TransportSend(transport) = error else {
        return false;
    };
    matches!(
        transport.error.downcast_ref::<HttpError>(),
        Some(StreamableHttpError::Client(e)) if e.is_connect()
    )
}

/// 清单写了 `mcp.tools`（允许列表）时只提供列表里的工具：调用不在列表里的和调用不存在的工具一样报错
/// （`call_plugin` 和确认策略视图都经 [`Upstreams::tools`] 拿工具）。列表里写了但上游没有的名字不拦别的
/// 工具，缺哪些写在问题里，由 `wonton__list_plugins` 的 `tools_error` 显示；上游整个列不出来时如实报
/// 原来的错。
fn allowlist(
    server: &McpServer,
    listed: Result<Vec<Tool>, String>,
) -> (Result<Vec<Tool>, String>, Option<String>) {
    let Some(declared) = &server.tools else {
        let problem = listed.as_ref().err().cloned();
        return (listed, problem);
    };
    match listed {
        Ok(tools) => {
            let names: std::collections::HashSet<&str> =
                tools.iter().map(|tool| tool.name.as_ref()).collect();
            let missing: Vec<&str> = declared
                .iter()
                .map(String::as_str)
                .filter(|name| !names.contains(name))
                .collect();
            let problem = (!missing.is_empty())
                .then(|| format!("清单声明的工具上游没有：{}", missing.join("、")));
            let provided: std::collections::HashSet<&str> =
                declared.iter().map(String::as_str).collect();
            let kept = tools
                .into_iter()
                .filter(|tool| provided.contains(tool.name.as_ref()))
                .collect();
            (Ok(kept), problem)
        }
        Err(why) => (Err(why), None),
    }
}

/// 列工具为什么失败。
enum ListFailure {
    Service(ServiceError),
    /// 插件回答了，回答不合规矩。
    Plugin(String),
}

/// 自己翻页：限页数、工具数和总字节数，见 [`MAX_TOOL_PAGES`]。
async fn list_pages(connection: &Connection, id: &PluginId) -> Result<Vec<Tool>, ListFailure> {
    let mut tools = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut bytes = 0;
    let mut cursor = None;
    for _ in 0..MAX_TOOL_PAGES {
        let params = PaginatedRequestParams::default().with_cursor(cursor);
        let page = connection
            .peer()
            .list_tools(Some(params))
            .await
            .map_err(ListFailure::Service)?;
        bytes += serde_json::to_vec(&page.tools).map_or(0, |page| page.len());
        tools.extend(page.tools);
        if tools.len() > MAX_TOOLS || bytes > MAX_TOOLS_BYTES {
            return Err(ListFailure::Plugin(format!(
                "插件 {id} 的工具列表超过上限（{MAX_TOOLS} 个工具、{MAX_TOOLS_BYTES} 字节）"
            )));
        }
        let Some(next) = page.next_cursor else {
            return Ok(tools);
        };
        if !seen.insert(next.clone()) {
            return Err(ListFailure::Plugin(format!(
                "插件 {id} 的工具列表翻页翻回了同一个 cursor"
            )));
        }
        cursor = Some(next);
    }
    Err(ListFailure::Plugin(format!(
        "插件 {id} 的工具列表超过 {MAX_TOOL_PAGES} 页"
    )))
}
