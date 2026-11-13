//! 唤醒标签：消息事件 ID → 链（ADR-0061 第 10 节"点名叫回沿用原链时按它找链"）。
//!
//! 标签由第 6/8 块登记：链上的 Bot 发进对话的消息，凭出箱回执的事件 ID 调 [`super::Chains::bind_event`]；
//! 桥报 start 时按触发消息查链，查到就沿用原链。只存内存：重启后链都转 `Ended`，标签随之失效。
//! 停链不摘标签——之后用这条链的消息开轮要拿到 `stopped`，而不是静默开新链；链结束才摘。

use super::{ChainLedgerError, LedgerState};
use wonton_protocol::{AgentChainStatus, BotRef};

pub(super) fn bind(
    state: &mut LedgerState,
    event_id: &str,
    chain_id: &str,
) -> Result<(), ChainLedgerError> {
    if !state.chains.contains_key(chain_id) {
        return Err(ChainLedgerError::UnknownChain(chain_id.to_owned()));
    }
    state.wake.insert(event_id.to_owned(), chain_id.to_owned());
    Ok(())
}

pub(super) fn lookup(state: &LedgerState, event_id: &str) -> Option<String> {
    let chain_id = state.wake.get(event_id)?;
    state
        .chains
        .contains_key(chain_id)
        .then(|| chain_id.clone())
}

/// 按（Bot、房间、话题）找它当时活的链：链上 Bot 发进同一对话的消息都挂回这条链（第 8 块）。
/// 已停的链不再接新消息——要接着做由人重新说一句开新链；已到上限的链照挂，
/// 让被点名的下一轮拿到 `limit_reached` 而不是绕开上限。同一 Bot 在同一对话有多条活链
/// （两个发起人各开了一条）时取最近活动的。
pub(super) fn chain_for_wake(
    state: &LedgerState,
    bot: &BotRef,
    room: &str,
    thread: Option<&str>,
) -> Option<String> {
    let mut best: Option<(u64, &str)> = None;
    for chain in state.chains.values() {
        if !matches!(
            chain.status,
            AgentChainStatus::Running | AgentChainStatus::LimitReached
        ) {
            continue;
        }
        if chain.origin.room_id != room || chain.origin.thread_id.as_deref() != thread {
            continue;
        }
        if !chain.bots.iter().any(|entry| &entry.bot == bot) {
            continue;
        }
        let tick = state.activity.get(&chain.chain).copied().unwrap_or(0);
        if best.is_none_or(|(kept, _)| tick > kept) {
            best = Some((tick, chain.chain.as_str()));
        }
    }
    best.map(|(_, chain)| chain.to_owned())
}

pub(super) fn drop_chain(state: &mut LedgerState, chain_id: &str) {
    state.wake.retain(|_, bound| bound != chain_id);
}
