//! Agent 链（ADR-0061 第 10 节）的壳入口：列表订阅和动作分派，逻辑在 [`crate::agent_chains`]。

#[cfg(feature = "ui-fake")]
use std::sync::atomic::Ordering;

use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{
    AgentChainAction, AgentChainsViewModel, AgentSettingsViewModel, CoreError, ProxySettings,
};

use super::*;
use crate::agent_chains::actions::{self, Effect};
use crate::agent_chains::apply_row;
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

    /// 链动作的单独分派入口（和 [`dispatch`](Self::dispatch) 并列，原因见 [`AgentChainAction`]）：
    /// 停链、改上限、改使用方并发。停链成功时列表里那一行马上变成已停。
    pub async fn dispatch_agent_chain(&self, action: AgentChainAction) -> Result<(), CoreError> {
        // 假数据入口（`new_ui_fake`）没有后端：动作不落到作用域，同 `dispatch`。
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Ok(());
        }
        let scope = self.scope();
        let (core_url, proxy) = connection(&scope)?;
        let effect = scope
            .run(async move { actions::perform(&core_url, &proxy, &action).await })
            .await?;
        if let Effect::Stopped(row) = effect {
            self.views()
                .agent_chains
                .send_modify(|view| apply_row(view, row));
        }
        Ok(())
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
fn connection(scope: &Scope) -> Result<(Url, ProxySettings), CoreError> {
    match (scope.home.core_url(), scope.home.view().proxy) {
        (Some(core_url), Some(proxy)) => Ok((core_url, proxy)),
        _ => Err(CoreError::TransportUnavailable {
            reason: l10n!(CATALOG, language(), "agent-api-not-connected"),
        }),
    }
}
