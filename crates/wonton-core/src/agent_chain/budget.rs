//! 轮数和花费的记账（ADR-0061 第 10 节）：轮数是链上所有会话的 `session/prompt` 加起来，
//! 花费是链上经模型网关的用量，按链记，token 另按 Bot 分账。
//! 到上限的那一下把链置 `LimitReached`；状态只从 `Running` 翻转一次，
//! "用到上限"事件由 [`StoredChain::limit_events`] 计数、经回调恰好发出一次。

use wonton_protocol::{AgentChainStatus, BotRef, ChainError, ChainLimitKind, ChainLimits};

use super::ChainLimitEvent;
use super::store::{BotSpend, StoredChain};

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

/// 记模型网关报来的用量：链合计和按 Bot 分账同时加。总 token 到上限的那一下翻转链状态；
/// 已停或已到上限的链只记账（用量是真的），不再翻转。
pub(super) fn record_tokens(
    chain: &mut StoredChain,
    bot: &BotRef,
    tokens: u64,
    amount: Option<f64>,
    events: &mut Vec<ChainLimitEvent>,
) {
    chain.tokens = chain.tokens.saturating_add(tokens);
    match chain
        .spend_by_bot
        .iter_mut()
        .find(|entry| &entry.bot == bot)
    {
        Some(entry) => entry.tokens = entry.tokens.saturating_add(tokens),
        None => chain.spend_by_bot.push(BotSpend {
            bot: bot.clone(),
            tokens,
        }),
    }
    chain.amount = match (chain.amount, amount) {
        (None, part) => part,
        (Some(total), Some(part)) => Some(total + part),
        (Some(_), None) => None,
    };
    if chain.status == AgentChainStatus::Running && chain.tokens >= chain.limits.max_tokens {
        hit_limit(chain, ChainLimitKind::Cost, events);
    }
}

/// "再给一份"：轮数、花费上限各加一份默认值，回到 `Running`。"先停着"是停链，走 `stop`。
pub(super) fn grant_another(chain: &mut StoredChain) {
    let defaults = ChainLimits::default();
    chain.limits.max_rounds += defaults.max_rounds;
    chain.limits.max_tokens += defaults.max_tokens;
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
