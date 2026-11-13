//! 交接只派发第一轮；结果留在这里供发起 Bot 只读查询，不叫醒调用方。

mod client;
mod consent;
mod dedupe;
mod run;
mod status;

pub(in crate::mcp) use status::Permission;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use tokio::sync::watch;
use wonton_protocol::{BotRef, ConfirmationOrigin, HandOffResult, HandOffSession};

use super::{Caller, ToolFailure};
use crate::gateway::Gateway;
use crate::gateway::agent_chains::AgentChains;
use wonton_protocol::{AgentChainStatus, HandOffRequest};

struct Context {
    gateway: Arc<Gateway>,
    chains: Arc<AgentChains>,
    from: BotRef,
    parent: String,
    chain: String,
    user: String,
    origin: ConfirmationOrigin,
    request: HandOffRequest,
}

pub(super) async fn dispatch(
    bus: &Arc<super::Bus>,
    caller: &Caller,
    request: HandOffRequest,
) -> OpenResult {
    let context = context(bus, caller, request)?;
    let (operation, fresh) = bus
        .handoffs
        .reserve(caller, &context.request, &context.chains)?;
    if fresh {
        let (bus, caller, operation) = (bus.clone(), caller.clone(), operation.clone());
        let completion = Completion(operation.clone());
        tokio::spawn(async move {
            let _completion = completion;
            if let Err(error) = run::start(&bus, &caller, context, &operation).await {
                operation.finish(Err(error));
            }
        });
    }
    operation.wait().await
}

fn context(
    bus: &super::Bus,
    caller: &Caller,
    request: HandOffRequest,
) -> Result<Context, ToolFailure> {
    let from = bot_of(caller)?;
    let parent = caller
        .session
        .as_ref()
        .ok_or_else(|| ToolFailure::new("permission_denied", "缺少调用方会话"))?;
    let chains = bus
        .agent_chains()
        .cloned()
        .ok_or_else(|| ToolFailure::new("service_unavailable", "调用链账本未启用"))?;
    let info = chains
        .ledger()
        .session_chain(parent)
        .filter(|info| info.bot == from)
        .ok_or_else(|| ToolFailure::new("permission_denied", "会话不属于调用方"))?;
    let row = chains
        .ledger()
        .row(&info.chain)
        .ok_or_else(|| ToolFailure::new("not_found", "调用链不存在"))?;
    if caller.chain.as_deref() != Some(&info.chain) {
        return Err(ToolFailure::new("permission_denied", "调用链与会话不符"));
    }
    match row.status {
        AgentChainStatus::Stopped => return Err(ToolFailure::new("stopped", "这条调用链已停下")),
        AgentChainStatus::LimitReached => {
            return Err(ToolFailure::new("limit_reached", "这条调用链已用到上限"));
        }
        AgentChainStatus::Ended | AgentChainStatus::Other => {
            return Err(ToolFailure::new("invalid_input", "这条调用链不在进行中"));
        }
        AgentChainStatus::Running => {}
    }
    let gateway = bus
        .handoff_gateway
        .get()
        .and_then(std::sync::Weak::upgrade)
        .ok_or_else(|| ToolFailure::new("service_unavailable", "交接入口未启用"))?;
    Ok(Context {
        gateway,
        chains,
        from,
        parent: parent.clone(),
        chain: info.chain,
        user: row.user,
        origin: row.origin,
        request,
    })
}

pub(super) fn conversation(
    bus: &super::Bus,
    caller: &Caller,
) -> Result<ConfirmationOrigin, ToolFailure> {
    let from = bot_of(caller)?;
    let session = caller
        .session
        .as_deref()
        .ok_or_else(|| ToolFailure::new("permission_denied", "缺少调用方会话"))?;
    let chains = bus
        .agent_chains()
        .ok_or_else(|| ToolFailure::new("service_unavailable", "调用链账本未启用"))?;
    let info = chains
        .ledger()
        .session_chain(session)
        .filter(|info| info.bot == from)
        .ok_or_else(|| ToolFailure::new("permission_denied", "会话不属于调用方"))?;
    chains
        .ledger()
        .row(&info.chain)
        .map(|row| row.origin)
        .ok_or_else(|| ToolFailure::new("not_found", "调用链不存在"))
}

pub(super) fn allows_relaxation(bus: &super::Bus, caller: &Caller) -> bool {
    let Ok(target) = bot_of(caller) else {
        return true;
    };
    let Some(chains) = bus.agent_chains() else {
        return true;
    };
    let state = bus.handoffs.state.lock().expect("交接表锁中毒");
    let live = || {
        state.sessions.values().filter(|entry| {
            entry.view.bot == target
                && chains
                    .ledger()
                    .session_chain(&entry.view.session)
                    .is_some_and(|session| session.live)
        })
    };
    if live().next().is_none() {
        return true;
    }
    let Some(account) = bus.plugins.bot_account(&caller.id, &target.bot) else {
        return false;
    };
    let Some((_, _, keeper)) = bus.plugins.bot_by_user(&account.user_id) else {
        return false;
    };
    let Ok(people) = bus.people() else {
        return false;
    };
    let keeper = format!("@{keeper}:{}", people.server_name);
    // 会话头是 Agent 自报的；省略或换一个自己的会话不能恢复放宽。
    live().all(|entry| {
        chains
            .ledger()
            .row(&entry.view.chain)
            .is_some_and(|row| row.user == keeper)
    })
}

fn chain_error(error: crate::agent_chain::ChainLedgerError) -> ToolFailure {
    use crate::agent_chain::ChainLedgerError as Ledger;
    use wonton_protocol::ChainError;
    let kind = match &error {
        Ledger::Rejected(rejection) => match rejection {
            ChainError::Stopped => "stopped",
            ChainError::LimitReached { .. } => "limit_reached",
            ChainError::Busy => "busy",
            ChainError::PermissionDenied => "permission_denied",
            ChainError::InvalidInput { .. } => "invalid_input",
            ChainError::Other => "other",
        },
        Ledger::Io { .. } | Ledger::Corrupt { .. } | Ledger::Serialize(_) => "storage",
        Ledger::UnknownChain(_) | Ledger::UnknownSession(_) | Ledger::UnknownCard(_) => "not_found",
        Ledger::AlreadyEnded { .. }
        | Ledger::NotAtLimit { .. }
        | Ledger::SessionsLive { .. }
        | Ledger::Invalid(_) => "invalid_input",
    };
    ToolFailure::new(kind, error.to_string())
}

pub(super) type OpenResult = Result<HandOffResult, ToolFailure>;

#[derive(Default)]
pub(super) struct Handoffs {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    requests: HashMap<dedupe::RequestKey, Arc<Operation>>,
    messages: HashMap<dedupe::MessageKey, Arc<Operation>>,
    sessions: BTreeMap<String, Entry>,
    permissions: HashMap<String, HashMap<String, u64>>,
}

struct Entry {
    from: BotRef,
    origin: ConfirmationOrigin,
    view: HandOffSession,
}

pub(super) struct Operation {
    result: watch::Sender<Option<OpenResult>>,
}

struct Completion(Arc<Operation>);

impl Drop for Completion {
    fn drop(&mut self) {
        self.0.result.send_if_modified(|result| {
            if result.is_some() {
                return false;
            }
            *result = Some(Err(ToolFailure::new("internal_error", "交接任务意外中止")));
            true
        });
    }
}

impl Operation {
    pub(super) fn finish(&self, result: OpenResult) {
        self.result.send_replace(Some(result));
    }

    pub(super) async fn wait(&self) -> OpenResult {
        let mut result = self.result.subscribe();
        loop {
            if let Some(value) = result.borrow().as_ref() {
                return value.clone();
            }
            if result.changed().await.is_err() {
                return Err(ToolFailure::new("internal_error", "交接任务没有给出结果"));
            }
        }
    }
}

fn bot_of(caller: &Caller) -> Result<BotRef, ToolFailure> {
    caller
        .bot
        .as_ref()
        .map(|bot| BotRef {
            plugin_id: caller.id.to_string(),
            bot: bot.bot.clone(),
        })
        .ok_or_else(|| ToolFailure::new("permission_denied", "交接工具只供已登记的 Bot 使用"))
}
