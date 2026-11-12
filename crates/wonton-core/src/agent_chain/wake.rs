//! 唤醒标签：消息事件 ID → 链（ADR-0061 第 10 节"点名叫回沿用原链时按它找链"）。
//!
//! 标签由第 6/8 块登记：链上的 Bot 发进对话的消息，凭出箱回执的事件 ID 调 [`super::Chains::bind_event`]；
//! 桥报 start 时按触发消息查链，查到就沿用原链。只存内存：重启后链都转 `Ended`，标签随之失效。
//! 停链不摘标签——之后用这条链的消息开轮要拿到 `stopped`，而不是静默开新链；链结束才摘。

use super::{ChainLedgerError, LedgerState};

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

pub(super) fn drop_chain(state: &mut LedgerState, chain_id: &str) {
    state.wake.retain(|_, bound| bound != chain_id);
}
