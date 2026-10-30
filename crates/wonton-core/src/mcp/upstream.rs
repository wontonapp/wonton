//! 总线连插件的 MCP 服务：每个插件一个客户端（Streamable HTTP，经插件网络），用到时连上，出错时丢掉，下次重连。

use std::collections::HashMap;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, Implementation,
    JsonObject, ProtocolVersion, Tool,
};
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientLifecycleMode, ClientServiceExt};
use tokio::sync::Mutex;

use crate::plugin::{McpServer, PluginId};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

struct Client {
    url: String,
    service: RunningService<RoleClient, ClientConfig>,
}

pub struct Upstreams {
    http: reqwest::Client,
    clients: Mutex<HashMap<PluginId, Client>>,
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
            .build()
            .expect("构造 MCP 客户端的 HTTP 客户端");
        Self {
            http,
            clients: Mutex::new(HashMap::new()),
        }
    }

    async fn peer(&self, server: &McpServer) -> Result<Peer<RoleClient>, String> {
        let mut clients = self.clients.lock().await;
        if let Some(client) = clients.get(&server.id)
            && client.url == server.url
            && !client.service.is_closed()
        {
            return Ok(client.service.peer().clone());
        }
        let transport = StreamableHttpClientTransport::with_client(
            self.http.clone(),
            StreamableHttpClientTransportConfig::with_uri(server.url.clone()),
        );
        // 用 initialize 握手（2025-11-25），不用 2026-07-28 的 server/discover：插件的 MCP 服务大多还不支持新协议，
        // supergateway 4.2.1 转 stdio 时收到 server/discover 就断开、不回应（实测），客户端会一直等下去。
        let info = ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("wonton-core", env!("CARGO_PKG_VERSION")),
        )
        .with_protocol_version(ProtocolVersion::V_2025_11_25);
        let lifecycle = ClientLifecycleMode::Initialize;
        let service = info
            .serve_with_lifecycle(transport, lifecycle)
            .await
            .map_err(|e| format!("连不上插件 {} 的 MCP 服务：{e}", server.id))?;
        let peer = service.peer().clone();
        let client = Client {
            url: server.url.clone(),
            service,
        };
        clients.insert(server.id.clone(), client);
        Ok(peer)
    }

    async fn forget(&self, id: &PluginId) {
        self.clients.lock().await.remove(id);
    }

    /// 插件现在提供的工具。
    pub async fn tools(&self, server: &McpServer) -> Result<Vec<Tool>, String> {
        let peer = self.peer(server).await?;
        match peer.list_all_tools().await {
            Ok(tools) => Ok(tools),
            Err(e) => {
                self.forget(&server.id).await;
                Err(format!("插件 {} 的 MCP 服务列不出工具：{e}", server.id))
            }
        }
    }

    /// 调用插件的工具。
    pub async fn call(
        &self,
        server: &McpServer,
        tool: &str,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let peer = self.peer(server).await?;
        let mut request = CallToolRequestParams::new(tool.to_owned());
        if let Some(arguments) = arguments {
            request = request.with_arguments(arguments);
        }
        match peer.call_tool(request).await {
            Ok(result) => Ok(result),
            Err(e) => {
                self.forget(&server.id).await;
                Err(format!("调用插件 {} 的工具 {tool} 失败：{e}", server.id))
            }
        }
    }

    /// 只留下还在运行的插件的客户端。
    pub async fn retain(&self, servers: &[McpServer]) {
        self.clients
            .lock()
            .await
            .retain(|id, client| servers.iter().any(|s| &s.id == id && s.url == client.url));
    }
}
