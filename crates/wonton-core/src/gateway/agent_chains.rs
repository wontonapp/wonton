//! `POST /agent/chains`：桥每轮开始、结束都在这里报（ADR-0061 第 10 节，接口在 `wonton-protocol` 的
//! `agent_chain`）。门和 `/agent/permissions` 一样在 [`super::mcp`] 后面，另外验 Bot 的 Matrix 令牌
//! （ADR-0083 第 5 节）：总线令牌 Agent 自己也拿得到，只认来源地址不够。
//!
//! 说话人必须是这个 Bot 听的人（ADR-0045 的名单：归属人和已成立的联系人，同归属人的另一个 Bot 也算），
//! 否则 403 `permission_denied`；说话人是 Bot 时这一轮靠唤醒标签接回原链（第 8 块）。
//!
//! 这里也放着链的服务：账本、出箱、设置和 `agent_acp` 的取消接口（第 5 块接上）。

use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use hyper::{Method, StatusCode};
use wonton_protocol::{BOT_TOKEN_HEADER, BotRef, ChainError, ChainTurnReport};

use super::Gateway;
use super::api::error_response;
use crate::agent_chain::ChainLimitEvent;
use crate::agent_chain::settings::AgentSettings;
use crate::agent_chain::{ChainLedgerError, Chains};
use crate::agent_outbox::Outbox;
use crate::mcp::Caller;
use crate::plugin::PluginId;

/// 请求体的上限：一轮的报告就是几个 ID，再大是调用方的错。
const MAX_BODY: usize = 64 * 1024;

/// `agent_acp` 连接上会话的取消（ADR-0061 第 10 节，第 5 块的网关接上）：停链时核心对每个经 `agent_acp`
/// 的会话发 `session/cancel`，网关自己把未决的权限请求回 `cancelled`。取消是异步的，接口不等它。
pub trait AgentAcpCancel: Send + Sync {
    fn cancel(&self, session: &str);
}

pub struct AgentChains {
    ledger: Arc<Chains>,
    outbox: Arc<Outbox>,
    settings: Arc<AgentSettings>,
    /// `agent_acp` 的取消方；第 5 块接上之前停链只停桥的轮次。
    acp: std::sync::OnceLock<Arc<dyn AgentAcpCancel>>,
}

impl AgentChains {
    pub fn new(ledger: Arc<Chains>, outbox: Arc<Outbox>, settings: Arc<AgentSettings>) -> Self {
        Self {
            ledger,
            outbox,
            settings,
            acp: std::sync::OnceLock::new(),
        }
    }

    pub fn ledger(&self) -> &Arc<Chains> {
        &self.ledger
    }

    pub fn outbox(&self) -> &Outbox {
        &self.outbox
    }

    pub fn settings(&self) -> &AgentSettings {
        &self.settings
    }

    /// 接上 `agent_acp` 的取消方（第 5 块）。接两次时保留先接上的。
    pub fn attach_acp_cancel(&self, cancel: Arc<dyn AgentAcpCancel>) {
        if self.acp.set(cancel).is_err() {
            tracing::warn!("agent_acp 的取消方接了两次，用先接上的");
        }
    }

    /// 停整条链（ADR-0061 第 10 节）：账本置 `Stopped` 并交出活会话和待答卡片；桥的轮次进这个 Bot 的出箱
    /// 发 `CancelTurn`，`agent_acp` 的会话交给网关取消。撤回卡片归总线（[`crate::mcp::Bus::stop_chain`]）。
    pub fn stop(
        &self,
        chain: &str,
    ) -> Result<crate::agent_chain::ChainStopOutcome, ChainLedgerError> {
        let outcome = self.ledger.stop(chain)?;
        for session in &outcome.bridge {
            // CancelTurn 不落盘，push 不会失败；失败了也只是这条取消发不出，链已经停了。
            if let Err(error) = self.outbox.push(
                &session.bot,
                wonton_protocol::OutboxKind::CancelTurn {
                    chain: chain.to_owned(),
                    session: session.session.clone(),
                },
            ) {
                tracing::warn!(%error, session = %session.session, "取消轮次没能排进出箱");
            }
        }
        if let Some(acp) = self.acp.get() {
            for session in outcome.agent_acp.iter().chain(&outcome.hand_off) {
                acp.cancel(&session.session);
            }
        }
        Ok(outcome)
    }
}

impl Gateway {
    #[must_use]
    pub fn with_agent_chains(mut self, chains: Option<Arc<AgentChains>>) -> Self {
        self.agent_chains = chains;
        self
    }

    /// 这个 Bot 听的人里有没有 `sender`（ADR-0045 的名单）：归属人本人、已成立的联系人，或者同一个归属人的
    /// 另一个 Bot（ADR-0083 第 4 节）。认不出这个 Bot 时是假。
    pub(crate) fn bot_listens_to(&self, bot: &BotRef, sender: &str) -> bool {
        let Ok(id) = PluginId::parse(&bot.plugin_id) else {
            return false;
        };
        let Some(account) = self.plugins.bot_account(&id, &bot.bot) else {
            return false;
        };
        // 归属人：私人 Bot 是这个人，共享 Bot 是主人。同一个归属人的另一个 Bot 也算（Bot 之间按名字叫）。
        let viewer = self.plugins.bot_by_user(sender);
        if let Some((_, _, keeper)) = &viewer
            && self
                .plugins
                .bot_by_user(&account.user_id)
                .is_some_and(|(_, _, own)| &own == keeper)
        {
            return true;
        }
        let keeper = keeper_of(&self.plugins, &account.user_id);
        let keeper_id = self.matrix.as_ref().map(|m| m.user_id(&keeper));
        if keeper_id.as_deref() == Some(sender) {
            return true;
        }
        self.contacts().is_some_and(|contacts| {
            contacts
                .allowed_users(&account.user_id)
                .iter()
                .any(|u| u == sender)
        })
    }
}

fn keeper_of(plugins: &crate::plugin::PluginManager, bot_user_id: &str) -> String {
    plugins
        .bot_by_user(bot_user_id)
        .map(|(_, _, keeper)| keeper)
        .unwrap_or_default()
}

/// 桥报一轮（开始、结束）。
pub(super) async fn report(gateway: &Gateway, caller: &Caller, req: Request) -> Response {
    let Some(chains) = &gateway.agent_chains else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "chains_unavailable",
            "调用链账本没接上",
        );
    };
    if req.method() != Method::POST {
        return error_response(StatusCode::METHOD_NOT_ALLOWED, "invalid_input", "只收 POST");
    }
    let Some(bot) = caller.bot.as_ref().map(|bot| BotRef {
        plugin_id: caller.id.to_string(),
        bot: bot.bot.clone(),
    }) else {
        return refused("这个插件没有 Bot 记录；调用链由桥按 Bot 报");
    };
    let Ok(plugin_id) = PluginId::parse(&bot.plugin_id) else {
        return refused("插件 ID 认不出来");
    };
    // Bot 的 Matrix 令牌：必须正是这个 Bot 的，别的 Bot 的、没有的都不行。
    let token = req
        .headers()
        .get(BOT_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let Some(user_id) = gateway.bot_user_of_token(token).await else {
        return refused("缺 Bot 的 Matrix 令牌，或者令牌认不出来");
    };
    let account = gateway.plugins.bot_account(&plugin_id, &bot.bot);
    if account.as_ref().map(|account| account.user_id.as_str()) != Some(user_id.as_str()) {
        return refused("Matrix 令牌不是这个 Bot 的");
    }
    let body = match to_bytes(req.into_body(), MAX_BODY).await {
        Ok(body) => body,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &format!("请求体读不出来：{e}"),
            );
        }
    };
    let report: ChainTurnReport = match serde_json::from_slice(&body) {
        Ok(report) => report,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &format!("请求体不是轮次报告：{e}"),
            );
        }
    };
    match report {
        ChainTurnReport::Start { ref sender, .. } => {
            if !gateway.bot_listens_to(&bot, sender) {
                return refused("说话的人不是这个 Bot 听的人");
            }
            // 说话人是 Bot 时链跟着叫回走（第 8 块）：消息带唤醒标签接回那条链；没有标签
            // （点名者不在链上，如自带 Matrix 的 Agent）按点名者的归属人开新链——链代表是
            // 归属人，不是 Bot 的 Matrix 账号。归属人记的是和别的链一样的完整 Matrix ID。
            let (mentioner, chain_user) = match gateway.plugins.bot_by_user(sender) {
                Some((id, bot, keeper)) => {
                    let Some(matrix) = &gateway.matrix else {
                        return refused("认不出归属人的 Matrix 账号");
                    };
                    (
                        Some(BotRef {
                            plugin_id: id.to_string(),
                            bot,
                        }),
                        matrix.user_id(&keeper),
                    )
                }
                None => (None, sender.clone()),
            };
            match chains
                .ledger()
                .start_bridge_turn(bot, report, mentioner, chain_user)
            {
                Ok(grant) => axum::Json(grant).into_response(),
                Err(error) => ledger_error(error),
            }
        }
        ChainTurnReport::End { session } => {
            chains.ledger().end_session(&session);
            axum::Json(serde_json::json!({})).into_response()
        }
    }
}

fn refused(message: &str) -> Response {
    let error = ChainError::PermissionDenied;
    let mut response = (StatusCode::FORBIDDEN, axum::Json(error)).into_response();
    response.headers_mut().insert(
        super::pages::ERROR_HEADER,
        hyper::header::HeaderValue::from_static("permission_denied"),
    );
    if !message.is_empty() {
        tracing::info!(%message, "调用链报告被拒");
        if let Ok(value) = hyper::header::HeaderValue::from_str(message) {
            response.headers_mut().insert("X-Wonton-Detail", value);
        }
    }
    response
}

/// 账本的拒绝按类别回：线上类别原样在响应体里（[`ChainError`]），状态码只给不认识类别的调用方能用的提示。
pub(super) fn ledger_error(error: ChainLedgerError) -> Response {
    let (status, kind, wire) = match &error {
        ChainLedgerError::Rejected(rejection) => match rejection {
            ChainError::LimitReached { .. } => (
                StatusCode::TOO_MANY_REQUESTS,
                "limit_reached",
                rejection.clone(),
            ),
            ChainError::Stopped => (StatusCode::CONFLICT, "stopped", rejection.clone()),
            ChainError::Busy => (StatusCode::TOO_MANY_REQUESTS, "busy", rejection.clone()),
            ChainError::PermissionDenied => (
                StatusCode::FORBIDDEN,
                "permission_denied",
                rejection.clone(),
            ),
            ChainError::InvalidInput { .. } => {
                (StatusCode::BAD_REQUEST, "invalid_input", rejection.clone())
            }
            ChainError::Other => (StatusCode::BAD_REQUEST, "other", rejection.clone()),
        },
        ChainLedgerError::UnknownChain(_) | ChainLedgerError::UnknownSession(_) => {
            (StatusCode::NOT_FOUND, "not_found", ChainError::Other)
        }
        ChainLedgerError::AlreadyEnded { .. } | ChainLedgerError::NotAtLimit { .. } => {
            (StatusCode::CONFLICT, "invalid_input", ChainError::Other)
        }
        ChainLedgerError::SessionsLive { .. } | ChainLedgerError::Invalid(_) => {
            (StatusCode::BAD_REQUEST, "invalid_input", ChainError::Other)
        }
        ChainLedgerError::UnknownCard(_) => (StatusCode::NOT_FOUND, "not_found", ChainError::Other),
        ChainLedgerError::Corrupt { .. }
        | ChainLedgerError::Io { .. }
        | ChainLedgerError::Serialize(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage",
            ChainError::Other,
        ),
    };
    tracing::warn!(%error, "调用链报告没做成");
    let mut response = (status, axum::Json(wire)).into_response();
    response.headers_mut().insert(
        super::pages::ERROR_HEADER,
        hyper::header::HeaderValue::from_static(kind),
    );
    response
}

/// 上限事件到账本的回调：发上限卡片（第 3 块）。回调同步，卡片在后台任务里等回答。
pub fn limit_handler(bus: &Arc<crate::mcp::Bus>) -> Arc<dyn Fn(&ChainLimitEvent) + Send + Sync> {
    let bus = bus.clone();
    Arc::new(move |event: &ChainLimitEvent| {
        crate::mcp::chain_card::spawn(bus.clone(), event.clone());
    })
}
