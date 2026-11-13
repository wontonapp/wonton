//! 每个使用方同时开着的 `agent_acp` 连接数（ADR-0061 第 10 节）：满了回 `busy`。计数在注册表里，
//! 注册表同时是 [`crate::gateway::agent_chains::AgentAcpCancel`] 的实现：按账本会话键取消对应 ACP 会话。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

/// 注册表：活连接按使用方计数、按连接键寻址。Gateway 持有；停链取消经 [`crate::gateway::agent_chains::AgentAcpCancel`]。
pub struct Registry {
    inner: Mutex<State>,
}

struct State {
    per_consumer: BTreeMap<String, u32>,
    connections: BTreeMap<String, mpsc::UnboundedSender<String>>,
}

impl Registry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(State {
                per_consumer: BTreeMap::new(),
                connections: BTreeMap::new(),
            }),
        })
    }

    /// 登记一条连接：占一个并发名额，返回的席位在连接结束时归还名额并摘除取消通道。
    /// 满了（≥ limit）返回 None，调用方回 `busy`。
    pub(super) fn register(
        self: &Arc<Self>,
        consumer: &str,
        connection: &str,
        cancel: mpsc::UnboundedSender<String>,
        limit: u32,
    ) -> Option<Registration> {
        let mut state = self.inner.lock().expect("agent_acp 注册表锁中毒");
        let live = state.per_consumer.entry(consumer.to_owned()).or_default();
        if *live >= limit {
            return None;
        }
        *live += 1;
        state.connections.insert(connection.to_owned(), cancel);
        Some(Registration {
            registry: self.clone(),
            consumer: consumer.to_owned(),
            connection: connection.to_owned(),
            sessions: Vec::new(),
        })
    }

    /// 停链的取消：连接键（账本里的 `agent_acp` 会话）→ 给连接任务发一个信号。
    pub(super) fn cancel(&self, connection: &str) {
        if let Ok(state) = self.inner.lock()
            && let Some(cancel) = state.connections.get(connection)
        {
            // 发不进去（任务已结束）就随它去：链已经停了，连接马上也会被账本记为结束。
            let _ = cancel.send(connection.to_owned());
        }
    }
}

/// 一条活连接在注册表里的席位。
pub(crate) struct Registration {
    registry: Arc<Registry>,
    consumer: String,
    connection: String,
    sessions: Vec<String>,
}

impl Registration {
    /// 交接的真实会话 ID 登记在停链接口上，但仍只占这条连接的一个并发名额。
    pub(crate) fn bind_session(&mut self, session: &str) -> bool {
        let mut state = self.registry.inner.lock().expect("agent_acp 注册表锁中毒");
        if state.connections.contains_key(session) {
            return false;
        }
        let Some(cancel) = state.connections.get(&self.connection).cloned() else {
            return false;
        };
        state.connections.insert(session.to_owned(), cancel);
        self.sessions.push(session.to_owned());
        true
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut state) = self.registry.inner.lock() {
            if let Some(live) = state.per_consumer.get_mut(&self.consumer) {
                *live = live.saturating_sub(1);
                if *live == 0 {
                    state.per_consumer.remove(&self.consumer);
                }
            }
            state.connections.remove(&self.connection);
            for session in &self.sessions {
                state.connections.remove(session);
            }
        }
    }
}
