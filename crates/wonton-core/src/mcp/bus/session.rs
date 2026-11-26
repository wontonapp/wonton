//! 会话与握手：MCP 服务、旧协议会话的通知、停链。

use super::*;

impl Bus {
    /// 停整条链（ADR-0061 第 10 节）：账本置 `Stopped`、轮次进他的出箱并撤回待答卡片，
    /// 链上还等着的卡片撤回（等待方拿到 `cancelled`）。
    pub async fn stop_chain(
        self: &Arc<Self>,
        chain: &str,
    ) -> Result<crate::agent_chain::ChainStopOutcome, crate::agent_chain::ChainLedgerError> {
        use crate::agent_chain::ChainLedgerError;
        let Some(chains) = self.agent_chains.get() else {
            return Err(ChainLedgerError::Invalid("调用链的服务没接上".into()));
        };
        let outcome = chains.stop(chain)?;
        chain_card::withdraw_cards(self, &outcome);
        Ok(outcome)
    }

    pub fn endpoint(self: &Arc<Self>) -> Endpoint {
        Endpoint {
            bus: self.clone(),
            service: self.service(),
        }
    }

    fn service(self: &Arc<Self>) -> McpService {
        let bus = self.clone();
        // Host 已经由网关校验过；Agent 不是浏览器，带 Origin 的请求一律拒绝。
        let config = StreamableHttpServerConfig::default()
            .with_allowed_hosts([PLUGIN_MCP_HOST])
            .enforce_origin_validation();
        // rmcp 按会话上的消息算空闲（默认 300 秒），等确认时没有消息，调用会被切断，Agent 拿不到结果。
        let mut sessions = LocalSessionManager::default();
        sessions.session_config.keep_alive =
            Some(self.confirmations.ttl() + SESSION_IDLE_AFTER_CARD);
        StreamableHttpService::new(
            move || Ok(server::Handler::new(bus.clone())),
            sessions.into(),
            config,
        )
    }

    /// 插件表变了：丢掉不再运行的插件的客户端，告诉旧协议的会话工具列表变了（新协议的经 `subscriptions/listen`，
    /// 见 [`server::Handler`]）。
    pub(super) fn watch_plugins(self: &Arc<Self>) {
        let bus = Arc::downgrade(self);
        let mut changes = self.plugins.changes();
        tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                let Some(bus) = bus.upgrade() else { break };
                bus.upstreams.retain(&bus.plugins.mcp_servers());
                bus.end_stopped_takeovers();
                let peers: Vec<_> = bus.peers.lock().expect("会话表锁中毒").clone();
                let mut alive = Vec::new();
                for peer in peers {
                    if peer.notify_tool_list_changed().await.is_ok() {
                        alive.push(peer);
                    }
                }
                *bus.peers.lock().expect("会话表锁中毒") = alive;
            }
        });
    }

    pub(super) fn track(&self, peer: Peer<RoleServer>) {
        self.peers.lock().expect("会话表锁中毒").push(peer);
    }
}
