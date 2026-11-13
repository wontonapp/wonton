//! 停链（ADR-0061 第 10 节）：置 `Stopped`，交出链上活的会话（按来源分）和带这条链、
//! 还没答的卡片；之后一切新登记都是 `stopped`。停是幂等的：已停的链再停一次返回空结果。

use wonton_protocol::AgentChainStatus;

use super::{
    ChainLedgerError, ChainSessionInfo, ChainSessionSource, ChainStopOutcome, LedgerState,
};

pub(super) fn stop(
    state: &mut LedgerState,
    chain_id: &str,
) -> Result<ChainStopOutcome, ChainLedgerError> {
    let Some(chain) = state.chains.get_mut(chain_id) else {
        return Err(ChainLedgerError::UnknownChain(chain_id.to_owned()));
    };
    match chain.status {
        AgentChainStatus::Ended => {
            return Err(ChainLedgerError::AlreadyEnded {
                chain: chain_id.to_owned(),
            });
        }
        AgentChainStatus::Stopped => return Ok(ChainStopOutcome::default()),
        _ => {}
    }
    chain.status = AgentChainStatus::Stopped;
    chain.limit_kind = None;
    bump_activity(state, chain_id);

    let mut outcome = ChainStopOutcome::default();
    for (session, entry) in state.sessions.iter_mut() {
        if entry.chain != chain_id || !entry.live {
            continue;
        }
        entry.live = false;
        let info = ChainSessionInfo {
            session: session.clone(),
            chain: chain_id.to_owned(),
            bot: entry.bot.clone(),
            layer: entry.layer,
            source: entry.source,
            parents: entry
                .path
                .parents
                .iter()
                .map(|parent| parent.session.clone())
                .collect(),
            connection: entry.connection,
            sender: entry.sender.clone(),
            live: false,
        };
        match entry.source {
            ChainSessionSource::Bridge => outcome.bridge.push(info),
            ChainSessionSource::AgentAcp => outcome.agent_acp.push(info),
            ChainSessionSource::HandOff => outcome.hand_off.push(info),
        }
    }
    // HashMap 的遍历序不定：按层和会话 ID 排稳，调用方和测试都拿到确定的顺序。
    for group in [
        &mut outcome.bridge,
        &mut outcome.agent_acp,
        &mut outcome.hand_off,
    ] {
        group.sort_by(|x, y| x.layer.cmp(&y.layer).then(x.session.cmp(&y.session)));
    }
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
