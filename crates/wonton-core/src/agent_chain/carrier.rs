//! ACP 链载体核对（ADR-0061 第 12 节）。会话归链、层数和成员只在账本里决定。

use wonton_protocol::{AgentChainStatus, BotRef, ChainCarrier, ChainError, ChainTurnGrant};

use super::{ChainLedgerError, ChainSessionSource, Chains, ancestry, layers, store, turns};

impl Chains {
    /// Preissue a connection chain without creating a live caller candidate. Gateway connection
    /// setup must use this instead of `open_acp_chain`; the latter remains a real root session.
    pub fn open_acp_connection(
        &self,
        user: &str,
        origin: wonton_protocol::ConfirmationOrigin,
        target: BotRef,
        connection: &str,
    ) -> Result<ChainTurnGrant, ChainLedgerError> {
        self.mutate(|state| {
            let grant = turns::open_chain(
                state,
                user.to_owned(),
                user,
                origin,
                store::ChainBotEntry {
                    bot: target,
                    layer: 1,
                },
                connection,
                ChainSessionSource::AgentAcp,
            )?;
            let entry = state.sessions.get_mut(connection).expect("new connection");
            entry.connection = true;
            entry.live = false;
            Ok(grant)
        })
    }

    /// Validate a new session's carrier. `connection` is the preissued connection identity;
    /// live real sessions only determine membership and caller ancestry, never placeholders.
    pub fn attach_carrier(
        &self,
        connection: &str,
        session: &str,
        consumer: &str,
        initiator: Option<&BotRef>,
        carrier: Option<&ChainCarrier>,
    ) -> Result<ChainTurnGrant, ChainLedgerError> {
        self.mutate(|state| {
            if state.sessions.contains_key(session) {
                return Err(ChainLedgerError::Invalid(format!("会话 {session} 已登记")));
            }
            let fallback = state
                .sessions
                .get(connection)
                .cloned()
                .ok_or_else(|| ChainLedgerError::UnknownSession(connection.to_owned()))?;
            let original = state.chains.get(&fallback.chain).expect("连接的链已登记");
            let Some(carrier) = carrier else {
                layers::reject_if_closed(original).map_err(ChainLedgerError::Rejected)?;
                if initiator == Some(&fallback.bot) {
                    return Err(ChainLedgerError::Rejected(ChainError::InvalidInput {
                        refusal: wonton_protocol::ChainRefusal::SelfCall,
                    }));
                }
                let grant = ChainTurnGrant {
                    chain: fallback.chain.clone(),
                    layer: fallback.layer,
                };
                ancestry::register(
                    state,
                    &fallback.chain,
                    session,
                    fallback.bot,
                    ChainSessionSource::AgentAcp,
                    fallback.layer,
                    Vec::new(),
                )?;
                state.dirty = true;
                return Ok(grant);
            };
            let inherited = state.chains.get(&carrier.id);
            if inherited.is_some_and(|chain| chain.status == AgentChainStatus::Stopped) {
                return Err(ChainLedgerError::Rejected(ChainError::Stopped));
            }
            let parents = ancestry::callers(state, &carrier.id, initiator, Some(consumer));
            let member = inherited.is_some_and(|chain| chain.status != AgentChainStatus::Ended)
                && !parents.is_empty();
            if !member {
                if initiator == Some(&fallback.bot) {
                    return Err(ChainLedgerError::Rejected(ChainError::InvalidInput {
                        refusal: wonton_protocol::ChainRefusal::SelfCall,
                    }));
                }
                let layer = layers::next_layer(0, carrier.depth, state.default_limits.max_depth)
                    .map_err(ChainLedgerError::Rejected)?;
                let user = original.user.clone();
                let origin = original.origin.clone();
                return turns::open_chain(
                    state,
                    user,
                    "",
                    origin,
                    store::ChainBotEntry {
                        bot: fallback.bot,
                        layer,
                    },
                    session,
                    ChainSessionSource::AgentAcp,
                );
            }
            let layer = ancestry::evaluate(
                state,
                &carrier.id,
                &fallback.bot,
                initiator,
                &parents,
                carrier.depth,
            )?;
            ancestry::register(
                state,
                &carrier.id,
                session,
                fallback.bot,
                ChainSessionSource::AgentAcp,
                layer,
                parents,
            )?;
            Ok(ChainTurnGrant {
                chain: carrier.id.clone(),
                layer,
            })
        })
    }
}
