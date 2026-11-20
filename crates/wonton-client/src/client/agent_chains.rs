//! Agent 链（ADR-0061 第 10 节）的壳入口：列表订阅和动作分派，逻辑在 [`crate::agent_chains`]。

use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{
    AgentChainsViewModel, AgentSettingsViewModel, CoreError, ProxySettings, TimelineTarget,
};

use super::*;
use crate::agent_chains::{actions, room_stop};
use crate::l10n::{CATALOG, language};
use crate::subscription::{AgentChainsSubscription, AgentStopSubscription};

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

    /// 订阅一条时间线的停止控件（[`RoomAgentStop`](wonton_protocol::RoomAgentStop)）：第一次 `next()` 立即返回
    /// 当前值，之后只在控件该换样子时返回。上一次停的结果不带进来。
    pub fn subscribe_agent_stop(&self, target: TimelineTarget) -> Arc<AgentStopSubscription> {
        let views = self.views();
        room_stop::forget_settled(&views.agent_stops, &target);
        Arc::new(AgentStopSubscription::new(
            views.agent_chains.subscribe(),
            views.agent_stops.subscribe(),
            target,
        ))
    }

    /// 设置“Agent”一页的当前值（层数、轮数上限）：改上限前要有当前值可读。
    pub async fn agent_settings(&self) -> Result<AgentSettingsViewModel, CoreError> {
        let scope = self.scope();
        let (core_url, proxy) = connection(&scope)?;
        scope
            .run(async move { actions::settings(&core_url, &proxy).await })
            .await
    }
}

impl ClientCore {
    /// [`AgentAction::StopTurns`](wonton_protocol::AgentAction::StopTurns)：结果都记在停止控件上，不同步返回。
    pub(super) fn stop_turns(&self, scope: &Scope, target: TimelineTarget) {
        let views = self.views();
        let Some(chains) = room_stop::begin(&views.agent_chains, &views.agent_stops, &target)
        else {
            return;
        };
        match connection(scope) {
            Ok(connection) => scope.spawn(room_stop::run(
                views.agent_chains.clone(),
                views.agent_stops.clone(),
                connection,
                target,
                chains,
            )),
            // 没连上家里，请求根本没发：是没停，不算不确定。
            Err(error) => {
                room_stop::record(
                    &views.agent_stops,
                    &target,
                    room_stop::Attempt::Failed(error),
                );
            }
        }
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
