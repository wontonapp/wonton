//! 停链（ADR-0061 第 10 节）：置 `Stopped`，交出链上活的轮次和带这条链、
//! 还没答的卡片；之后一切新登记都是 `stopped`。停是幂等的：已停的链再停一次，结果也照给。
//! 已经停过的链再停一次也要把当时还没答的卡片再交出去：停的时候卡片还没挂上（分发和停撞在一起），
//! 或者上一次停得半截，卡片就没人撤、一直等人答。
//! 做完的链也能停（用户按下停止钮时最后一轮可能刚好报了 End），改成 `Stopped`，之后不再能点名叫回。

use wonton_protocol::AgentChainStatus;

use super::{ChainLedgerError, ChainSessionInfo, ChainStopOutcome, LedgerState};

pub(super) fn stop(
    state: &mut LedgerState,
    chain_id: &str,
) -> Result<ChainStopOutcome, ChainLedgerError> {
    let Some(chain) = state.chains.get_mut(chain_id) else {
        return Err(ChainLedgerError::UnknownChain(chain_id.to_owned()));
    };
    if chain.status != AgentChainStatus::Stopped {
        chain.status = AgentChainStatus::Stopped;
        chain.limit_kind = None;
        bump_activity(state, chain_id);
    }

    let mut outcome = ChainStopOutcome::default();
    for (session, entry) in state.sessions.iter_mut() {
        if entry.chain != chain_id || !entry.live {
            continue;
        }
        entry.live = false;
        outcome.turns.push(ChainSessionInfo {
            session: session.clone(),
            chain: chain_id.to_owned(),
            bot: entry.bot.clone(),
            layer: entry.layer,
            sender: entry.sender.clone(),
            live: false,
        });
    }
    // HashMap 的遍历序不定：按层和会话 ID 排稳，调用方和测试都拿到确定的顺序。
    outcome
        .turns
        .sort_by(|x, y| x.layer.cmp(&y.layer).then(x.session.cmp(&y.session)));
    outcome.cards = state
        .cards
        .iter()
        .filter(|(_, card)| card.chain == chain_id && !card.answered)
        .map(|(id, _)| id.clone())
        .collect();
    outcome.cards.sort();
    state.cards.retain(|_, card| card.chain != chain_id);
    Ok(outcome)
}

pub(super) fn bump_activity(state: &mut LedgerState, chain_id: &str) {
    state.tick += 1;
    state.activity.insert(chain_id.to_owned(), state.tick);
}
