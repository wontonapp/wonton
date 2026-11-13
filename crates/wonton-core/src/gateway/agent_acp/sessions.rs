//! ACP 会话到核心账本键的映射；链归属和层数只由 agent_chain 决定。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};
use wonton_protocol::{ACP_CHAIN_META_KEY, BotRef};

use crate::agent_chain::{ChainLedgerError, Chains};

use super::{concurrency::Registration, frames};

pub(super) struct Sessions {
    ledger: Arc<Chains>,
    connection: String,
    consumer: String,
    initiator: BotRef,
    pending: Vec<PendingSession>,
    live: HashMap<String, String>,
}

struct PendingSession {
    id: Value,
    ledger: String,
    restored: Option<String>,
}

impl Sessions {
    pub(super) fn new(
        ledger: Arc<Chains>,
        connection: String,
        consumer: String,
        initiator: BotRef,
    ) -> Self {
        Self {
            ledger,
            connection,
            consumer,
            initiator,
            pending: Vec::new(),
            live: HashMap::new(),
        }
    }

    pub(super) fn prepare_new(
        &mut self,
        frame: &mut Value,
        registration: &mut Registration,
    ) -> Result<(), ChainLedgerError> {
        let id = frame
            .get("id")
            .filter(|id| !id.is_null())
            .ok_or_else(invalid)?
            .clone();
        if self.pending.iter().any(|pending| pending.id == id) {
            return Err(invalid());
        }
        let carrier = frames::carrier(frame)?;
        let params = frame
            .get_mut("params")
            .and_then(Value::as_object_mut)
            .ok_or_else(invalid)?;
        let meta = params
            .entry("_meta")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(invalid)?;
        let session = super::connection_key();
        let grant = self.ledger.attach_carrier(
            &self.connection,
            &session,
            &self.consumer,
            Some(&self.initiator),
            carrier.as_ref(),
        )?;
        meta.insert(
            ACP_CHAIN_META_KEY.to_owned(),
            json!({"id": grant.chain, "depth": grant.layer}),
        );
        registration.bind_session(&session);
        self.pending.push(PendingSession {
            id,
            ledger: session,
            restored: None,
        });
        Ok(())
    }

    pub(super) fn prepare_load(
        &mut self,
        frame: &Value,
        registration: &mut Registration,
    ) -> Result<(), ChainLedgerError> {
        let id = frame
            .get("id")
            .filter(|id| !id.is_null())
            .ok_or_else(invalid)?;
        let remote = frame["params"]["sessionId"].as_str().ok_or_else(invalid)?;
        if self.pending.iter().any(|pending| &pending.id == id) {
            return Err(invalid());
        }
        let session = super::connection_key();
        self.ledger.attach_carrier(
            &self.connection,
            &session,
            &self.consumer,
            Some(&self.initiator),
            None,
        )?;
        registration.bind_session(&session);
        self.pending.push(PendingSession {
            id: id.clone(),
            ledger: session,
            restored: Some(remote.to_owned()),
        });
        Ok(())
    }

    pub(super) fn observe_response(&mut self, frame: &Value) -> Option<String> {
        if frame.get("method").is_some() {
            return None;
        }
        let id = frame.get("id")?;
        let index = self.pending.iter().position(|pending| &pending.id == id)?;
        let PendingSession {
            ledger: session,
            restored,
            ..
        } = self.pending.swap_remove(index);
        let remote = frame["result"]["sessionId"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| frame.get("result").and(restored));
        let Some(remote) = remote else {
            self.ledger.end_session(&session);
            return None;
        };
        let stopped = self
            .ledger
            .session_chain(&session)
            .is_some_and(|info| !info.live);
        if let Some(previous) = self.live.insert(remote.clone(), session) {
            self.ledger.end_session(&previous);
        }
        stopped.then_some(remote)
    }

    pub(super) fn prepare_prompt(&self, frame: &mut Value) -> Result<bool, ChainLedgerError> {
        let remote = frame["params"]["sessionId"].as_str().ok_or_else(invalid)?;
        let session = self
            .live
            .get(remote)
            .ok_or_else(|| ChainLedgerError::UnknownSession(remote.to_owned()))?;
        self.ledger.begin_round(session)?;
        // 使用方不能在后续轮次中把已登记会话改挂到另一条链。
        Ok(frame["params"]["_meta"]
            .as_object_mut()
            .is_some_and(|meta| meta.remove(ACP_CHAIN_META_KEY).is_some()))
    }

    pub(super) fn cancelling<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> {
        self.live
            .iter()
            .filter_map(move |(remote, session)| (session == key).then_some(remote.as_str()))
    }
}

impl Drop for Sessions {
    fn drop(&mut self) {
        self.ledger.end_session(&self.connection);
        for session in self
            .live
            .values()
            .chain(self.pending.iter().map(|pending| &pending.ledger))
        {
            self.ledger.end_session(session);
        }
    }
}

fn invalid() -> ChainLedgerError {
    ChainLedgerError::Invalid("invalid ACP session request".to_owned())
}
