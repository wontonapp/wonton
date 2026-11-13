//! Immutable parent snapshots survive bridge session-key reuse. Bot-level APIs deliberately
//! union all live matching caller paths; handoff callers should use the exact-session APIs.

use std::sync::Arc;

use wonton_protocol::{BotRef, ChainError, ChainRefusal};

use super::{
    ChainLedgerError, ChainSessionSource, Chains, LedgerState, SessionEntry, layers, stop,
};

#[derive(Debug)]
pub(super) struct SessionPath {
    pub session: String,
    pub bot: BotRef,
    pub layer: u32,
    pub parents: Vec<Arc<SessionPath>>,
}

impl SessionPath {
    fn contains(&self, bot: &BotRef) -> bool {
        &self.bot == bot || self.parents.iter().any(|parent| parent.contains(bot))
    }
}

pub(super) fn callers(
    state: &LedgerState,
    chain: &str,
    initiator: Option<&BotRef>,
    plugin: Option<&str>,
) -> Vec<Arc<SessionPath>> {
    state
        .sessions
        .values()
        .filter(|entry| {
            entry.chain == chain
                && entry.live
                && !entry.connection
                && match initiator {
                    Some(bot) => &entry.bot == bot,
                    None => plugin.is_some_and(|plugin| entry.bot.plugin_id == plugin),
                }
        })
        .map(|entry| Arc::clone(&entry.path))
        .collect()
}

pub(super) fn evaluate(
    state: &LedgerState,
    chain_id: &str,
    target: &BotRef,
    initiator: Option<&BotRef>,
    parents: &[Arc<SessionPath>],
    depth: u32,
) -> Result<u32, ChainLedgerError> {
    let chain = state
        .chains
        .get(chain_id)
        .ok_or_else(|| ChainLedgerError::UnknownChain(chain_id.to_owned()))?;
    layers::reject_if_closed(chain).map_err(ChainLedgerError::Rejected)?;
    if initiator == Some(target) || parents.iter().any(|parent| &parent.bot == target) {
        return Err(ChainLedgerError::Rejected(ChainError::InvalidInput {
            refusal: ChainRefusal::SelfCall,
        }));
    }
    if parents.iter().any(|parent| parent.contains(target)) {
        return Err(ChainLedgerError::Rejected(ChainError::InvalidInput {
            refusal: ChainRefusal::Loop,
        }));
    }
    if initiator.is_some() && parents.is_empty() {
        return Err(ChainLedgerError::Invalid(
            "caller has no live session in chain".into(),
        ));
    }
    let parent_layer = parents.iter().map(|parent| parent.layer).max().unwrap_or(0);
    layers::next_layer(parent_layer, depth, chain.limits.max_depth)
        .map_err(ChainLedgerError::Rejected)
}

pub(super) fn register(
    state: &mut LedgerState,
    chain_id: &str,
    session: &str,
    bot: BotRef,
    source: ChainSessionSource,
    layer: u32,
    parents: Vec<Arc<SessionPath>>,
) -> Result<u32, ChainLedgerError> {
    if source == ChainSessionSource::Bridge {
        return Err(ChainLedgerError::Invalid(
            "桥的会话经 start_bridge_turn 登记".into(),
        ));
    }
    if let Some(existing) = state.sessions.get_mut(session) {
        if existing.chain == chain_id && existing.bot == bot && !existing.connection {
            existing.live = true;
            state.dirty = true;
            return Ok(existing.layer);
        }
        return Err(ChainLedgerError::Invalid(format!(
            "会话 {session} 已登记在别的链上"
        )));
    }
    layers::admit_bot(
        state.chains.get_mut(chain_id).expect("validated chain"),
        &bot,
        layer,
    );
    let path = Arc::new(SessionPath {
        session: session.to_owned(),
        bot: bot.clone(),
        layer,
        parents,
    });
    state.sessions.insert(
        session.to_owned(),
        SessionEntry {
            chain: chain_id.to_owned(),
            bot,
            layer,
            source,
            path,
            connection: false,
            sender: None,
            live: true,
        },
    );
    state.dirty = true;
    stop::bump_activity(state, chain_id);
    Ok(layer)
}

fn existing_session(
    state: &mut LedgerState,
    chain_id: &str,
    session: &str,
    bot: &BotRef,
    source: ChainSessionSource,
) -> Result<Option<u32>, ChainLedgerError> {
    if source == ChainSessionSource::Bridge {
        return Err(ChainLedgerError::Invalid(
            "桥的会话经 start_bridge_turn 登记".into(),
        ));
    }
    let chain = state
        .chains
        .get(chain_id)
        .ok_or_else(|| ChainLedgerError::UnknownChain(chain_id.to_owned()))?;
    layers::reject_if_closed(chain).map_err(ChainLedgerError::Rejected)?;
    match state.sessions.get_mut(session) {
        Some(entry) if entry.chain == chain_id && &entry.bot == bot && !entry.connection => {
            entry.live = true;
            state.dirty = true;
            Ok(Some(entry.layer))
        }
        Some(_) => Err(ChainLedgerError::Invalid(format!(
            "会话 {session} 已登记在别的链上"
        ))),
        None => Ok(None),
    }
}

fn exact_parent<'a>(
    state: &'a LedgerState,
    session: &str,
) -> Result<&'a SessionEntry, ChainLedgerError> {
    let parent = state
        .sessions
        .get(session)
        .ok_or_else(|| ChainLedgerError::UnknownSession(session.to_owned()))?;
    let chain = state.chains.get(&parent.chain).expect("registered chain");
    layers::reject_if_closed(chain).map_err(ChainLedgerError::Rejected)?;
    if !parent.live || parent.connection {
        return Err(ChainLedgerError::Invalid(format!(
            "会话 {session} 不是活的调用方"
        )));
    }
    Ok(parent)
}

impl Chains {
    /// Ambiguous Bot caller: union all its live session ancestry. No initiator admits a root
    /// at layer 1, without inventing a parent or consulting unrelated branches.
    pub fn preview_attach(
        &self,
        chain_id: &str,
        target: &BotRef,
        initiator: Option<&BotRef>,
    ) -> Result<u32, ChainLedgerError> {
        self.read(|state| {
            let parents = callers(state, chain_id, initiator, None);
            evaluate(state, chain_id, target, initiator, &parents, 0)
        })
    }

    /// Register against all live sessions of the ambiguous Bot caller. Only the same session
    /// key is reconnect-idempotent; distinct sessions of the same target remain independent.
    pub fn register_session(
        &self,
        chain_id: &str,
        session: &str,
        bot: BotRef,
        initiator: Option<&BotRef>,
        source: ChainSessionSource,
    ) -> Result<u32, ChainLedgerError> {
        self.mutate(|state| {
            if let Some(layer) = existing_session(state, chain_id, session, &bot, source)? {
                return Ok(layer);
            }
            let parents = callers(state, chain_id, initiator, None);
            let layer = evaluate(state, chain_id, &bot, initiator, &parents, 0)?;
            register(state, chain_id, session, bot, source, layer, parents)
        })
    }

    /// Exact-parent handoff preflight. The chain and layer come from this live caller session.
    pub fn preview_attach_from_session(
        &self,
        parent_session: &str,
        target: &BotRef,
    ) -> Result<u32, ChainLedgerError> {
        self.read(|state| {
            let parent = exact_parent(state, parent_session)?;
            evaluate(
                state,
                &parent.chain,
                target,
                Some(&parent.bot),
                std::slice::from_ref(&parent.path),
                0,
            )
        })
    }

    /// Register a child of one exact live caller, snapshotting its path before keys can be reused.
    /// Rechecks preflight under the mutation lock; callers cancel the remote session on failure.
    pub fn register_session_from_parent(
        &self,
        parent_session: &str,
        session: &str,
        bot: BotRef,
        source: ChainSessionSource,
    ) -> Result<u32, ChainLedgerError> {
        self.mutate(|state| {
            let parent = exact_parent(state, parent_session)?;
            let chain = parent.chain.clone();
            let parents = vec![Arc::clone(&parent.path)];
            if let Some(layer) = existing_session(state, &chain, session, &bot, source)? {
                return Ok(layer);
            }
            let layer = evaluate(state, &chain, &bot, Some(&parents[0].bot), &parents, 0)?;
            register(state, &chain, session, bot, source, layer, parents)
        })
    }
}
