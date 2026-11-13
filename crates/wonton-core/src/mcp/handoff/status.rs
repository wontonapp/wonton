use std::sync::Arc;

use wonton_protocol::{AgentChainStatus, BotRef, ConfirmationOrigin, HandOffSession, HandOffState};

use super::{Entry, Handoffs, bot_of};
use crate::mcp::{Bus, Caller, ToolFailure};

impl Handoffs {
    pub(super) fn opened(&self, from: BotRef, origin: ConfirmationOrigin, session: HandOffSession) {
        self.state.lock().expect("交接表锁中毒").sessions.insert(
            session.session.clone(),
            Entry {
                from,
                origin,
                view: session,
            },
        );
    }

    pub(super) fn text(&self, session: &str, text: &str) {
        let mut state = self.state.lock().expect("交接表锁中毒");
        if let Some(entry) = state.sessions.get_mut(session) {
            let tail = text.char_indices().rev().nth(1999).map(|(start, _)| start);
            if let Some(start) = tail {
                entry.view.summary.clear();
                entry.view.summary.push_str(&text[start..]);
            } else {
                entry.view.summary.push_str(text);
                if let Some((last, _)) = entry.view.summary.char_indices().rev().nth(1999) {
                    entry.view.summary.drain(..last);
                }
            }
            entry.view.last_activity_at = crate::peers::now_ms();
        }
    }

    pub(super) fn ended(&self, session: &str, reason: Option<String>) {
        if let Some(entry) = self
            .state
            .lock()
            .expect("交接表锁中毒")
            .sessions
            .get_mut(session)
        {
            entry.view.status = if reason.is_some() {
                HandOffState::Ended
            } else {
                HandOffState::Failed
            };
            entry.view.stop_reason = reason;
            entry.view.last_activity_at = crate::peers::now_ms();
        }
    }

    pub(in crate::mcp) fn list(
        &self,
        bus: &Bus,
        caller: &Caller,
        session: Option<&str>,
        origin: &ConfirmationOrigin,
    ) -> Result<Vec<HandOffSession>, ToolFailure> {
        let from = bot_of(caller)?;
        let state = self.state.lock().expect("交接表锁中毒");
        if let Some(session) = session {
            let entry = state
                .sessions
                .get(session)
                .ok_or_else(|| ToolFailure::new("not_found", "没有这个交接会话"))?;
            if entry.from != from {
                return Err(ToolFailure::new(
                    "permission_denied",
                    "只能查询自己交出去的会话",
                ));
            }
        }
        let now = crate::peers::now_ms();
        Ok(state
            .sessions
            .values()
            .filter(|entry| {
                entry.from == from
                    && match session {
                        Some(session) => entry.view.session == session,
                        None => &entry.origin == origin,
                    }
            })
            .map(|entry| {
                let mut view = entry.view.clone();
                let chain = bus
                    .agent_chains()
                    .and_then(|chains| chains.ledger().row(&view.chain));
                match chain.map(|row| row.status) {
                    Some(AgentChainStatus::Stopped) => view.status = HandOffState::Stopped,
                    Some(AgentChainStatus::LimitReached) => {
                        view.status = HandOffState::LimitReached
                    }
                    _ if view.status == HandOffState::Running
                        && state
                            .permissions
                            .get(&view.session)
                            .is_some_and(|cards| cards.values().any(|until| *until > now)) =>
                    {
                        view.status = HandOffState::WaitingPermission;
                    }
                    _ => {}
                }
                view
            })
            .collect())
    }

    pub(in crate::mcp) fn permission(
        self: &Arc<Self>,
        session: &str,
        card: &str,
        expires: u64,
    ) -> Permission {
        let mut state = self.state.lock().expect("交接表锁中毒");
        state
            .permissions
            .entry(session.to_owned())
            .or_default()
            .insert(card.to_owned(), expires);
        if let Some(entry) = state.sessions.get_mut(session) {
            entry.view.last_activity_at = crate::peers::now_ms();
        }
        Permission {
            handoffs: self.clone(),
            session: session.to_owned(),
            card: card.to_owned(),
        }
    }
}

pub(in crate::mcp) struct Permission {
    handoffs: Arc<Handoffs>,
    session: String,
    card: String,
}

impl Drop for Permission {
    fn drop(&mut self) {
        let mut state = self.handoffs.state.lock().expect("交接表锁中毒");
        if let Some(cards) = state.permissions.get_mut(&self.session) {
            cards.remove(&self.card);
            if cards.is_empty() {
                state.permissions.remove(&self.session);
            }
        }
        if let Some(entry) = state.sessions.get_mut(&self.session) {
            entry.view.last_activity_at = crate::peers::now_ms();
        }
    }
}
