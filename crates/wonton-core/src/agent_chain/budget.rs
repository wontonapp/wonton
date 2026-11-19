//! 轮数的记账（ADR-0061 第 10 节）：轮数是链上所有轮次加起来。
//! 到上限的那一下把链置 `LimitReached`；状态只从 `Running` 翻转一次，
//! "用到上限"事件由 [`StoredChain::limit_events`] 计数、经回调恰好发出一次。

use wonton_protocol::{AgentChainStatus, ChainError, ChainLimitKind, ChainLimits};

use super::ChainLimitEvent;
use super::store::StoredChain;

/// 记一轮。到轮数上限的那一下翻转链状态并返回拒绝；这一轮不计入、不发给 Agent。
pub(super) fn count_round(
    chain: &mut StoredChain,
    events: &mut Vec<ChainLimitEvent>,
) -> Result<(), ChainError> {
    let next = chain.rounds + 1;
    if next > chain.limits.max_rounds {
        return Err(hit_limit(chain, ChainLimitKind::Rounds, events));
    }
    chain.rounds = next;
    Ok(())
}

/// "再给一份"：轮数上限加一份默认值，回到 `Running`。"先停着"是停链，走 `stop`。
pub(super) fn grant_another(chain: &mut StoredChain) {
    chain.limits.max_rounds += ChainLimits::default().max_rounds;
    chain.status = AgentChainStatus::Running;
    chain.limit_kind = None;
}

fn hit_limit(
    chain: &mut StoredChain,
    kind: ChainLimitKind,
    events: &mut Vec<ChainLimitEvent>,
) -> ChainError {
    chain.status = AgentChainStatus::LimitReached;
    chain.limit_kind = Some(kind);
    chain.limit_events += 1;
    events.push(ChainLimitEvent {
        chain: chain.chain.clone(),
        kind,
    });
    ChainError::LimitReached { kind }
}
