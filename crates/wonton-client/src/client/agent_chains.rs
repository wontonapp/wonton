//! Agent 链（ADR-0061 第 10 节）的壳入口：列表订阅和动作分派，逻辑在 [`crate::agent_chains`]。

use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{AgentChainsViewModel, AgentSettingsViewModel, CoreError, ProxySettings};

use super::*;
use crate::agent_chains::actions;
use crate::l10n::{CATALOG, language};
use crate::subscription::AgentChainsSubscription;

#[uniffi::export]
impl ClientCore {
    /// 当前的 Agent 链列表，用于首帧渲染。
    pub fn agent_chains(&self) -> AgentChainsViewModel {
        self.views().agent_chains.borrow().clone()
    }

    /// 订阅 Agent 链列表，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_agent_chains(&self) -> Arc<AgentChainsSubscription> {
        Arc::new(AgentChainsSubscription::new(
            self.views().agent_chains.subscribe(),
        ))
    }

    /// 设置“Agent”一页的当前值（层数、轮数、花费上限）：改上限前要有当前值可读。
    pub async fn agent_settings(&self) -> Result<AgentSettingsViewModel, CoreError> {
        let scope = self.scope();
        let (core_url, proxy) = connection(&scope)?;
        scope
            .run(async move { actions::settings(&core_url, &proxy).await })
            .await
    }
}

/// 链的请求跟别的核心接口一样走本地代理；没连上家里服务器时如实失败。
pub(super) fn connection(scope: &Scope) -> Result<(Url, ProxySettings), CoreError> {
    match (scope.home.core_url(), scope.home.view().proxy) {
        (Some(core_url), Some(proxy)) => Ok((core_url, proxy)),
        _ => Err(CoreError::TransportUnavailable {
            reason: l10n!(CATALOG, language(), "agent-api-not-connected"),
        }),
    }
}
