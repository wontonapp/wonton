use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rmcp::model::{ClientCapabilities, ClientConfig, Implementation, ProtocolVersion};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientLifecycleMode, ClientServiceExt};
use tokio::time::Instant;

use super::super::bounded_http::BoundedHttp;
use super::Upstreams;
use super::limits::MAX_TIMEOUTS;
use crate::plugin::{McpHeaders, McpServer, PluginId};

pub(super) type Service = RunningService<RoleClient, ClientConfig>;

/// 一条连接。正在跑的调用各持有一份：连接从表里丢掉时它们还能跑完，除非这条会话被换掉（见模块说明）。
pub(super) struct Connection {
    service: Service,
    /// 卡死被换掉了：这之后结束的调用不是连接断了。
    pub(super) replaced: AtomicBool,
    /// 正在跑的 `tools/call`、`resources/read` 什么时候开始的。
    calls: Mutex<Vec<Instant>>,
}

pub(super) struct Calling<'a> {
    connection: &'a Connection,
    started: Instant,
}

/// 一个插件的连接。
pub(in crate::mcp) struct Slot {
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
    pub(super) tools_problem: Option<String>,
}

impl Connection {
    pub(super) fn new(service: Service) -> Self {
        Self {
            service,
            replaced: AtomicBool::new(false),
            calls: Mutex::default(),
        }
    }

    pub(super) fn peer(&self) -> &rmcp::service::Peer<RoleClient> {
        self.service.peer()
    }

    pub(super) fn calling(&self, started: Instant) -> Calling<'_> {
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

impl Drop for Calling<'_> {
    fn drop(&mut self) {
        let mut calls = self.connection.calls.lock().expect("调用表锁中毒");
        if let Some(at) = calls.iter().position(|call| *call == self.started) {
            calls.swap_remove(at);
        }
    }
}

impl Slot {
    pub(super) fn new(url: String, headers: McpHeaders) -> Self {
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

impl Upstreams {
    /// 还能用的连接；没有时给出握手要持有的锁。地址或者请求头（密钥换过了）和现在的不一样就换新槽。
    pub(super) fn live(
        &self,
        server: &McpServer,
    ) -> Result<Arc<Connection>, Arc<tokio::sync::Mutex<()>>> {
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

    pub(super) async fn connection(&self, server: &McpServer) -> Result<Arc<Connection>, String> {
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
            Implementation::new("wonton-core", crate::build_info::version()),
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
    pub(super) fn forget(&self, id: &PluginId, broken: &Arc<Connection>) {
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
    pub(super) fn answered(&self, id: &PluginId, used: &Arc<Connection>) {
        let mut clients = self.clients.lock().expect("MCP 客户端表锁中毒");
        if let Some(slot) = clients.get_mut(id)
            && slot.current.as_ref().is_some_and(|c| Arc::ptr_eq(c, used))
        {
            slot.timeouts = 0;
            slot.counted_until = Instant::now();
        }
    }

    /// 这条连接上有一个 `started` 开始的请求没等到回答：够多次就换掉，见模块说明。
    pub(super) fn timed_out(&self, id: &PluginId, used: &Arc<Connection>, started: Instant) {
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
