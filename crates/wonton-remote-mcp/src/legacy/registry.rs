use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::watch;

use super::Session;
use crate::headers::RemoteRequest;

#[derive(Default)]
pub(crate) struct Sessions(Arc<Mutex<Registry>>);

#[derive(Default)]
pub(super) struct Registry {
    sessions: HashMap<String, Arc<Session>>,
    openings: HashMap<u64, Arc<OpeningState>>,
    next_opening: u64,
}

struct OpeningState {
    plugin: String,
    cancelled: watch::Sender<bool>,
    session: Mutex<Option<Weak<Session>>>,
}

pub(crate) struct Opening {
    id: u64,
    registry: Arc<Mutex<Registry>>,
    state: Arc<OpeningState>,
}

impl Sessions {
    pub fn begin(&self, plugin: &str) -> Opening {
        let mut registry = self.0.lock().expect("sessions lock");
        let id = registry.next_opening;
        registry.next_opening = id.checked_add(1).expect("opening ID exhausted");
        let state = Arc::new(OpeningState {
            plugin: plugin.to_owned(),
            cancelled: watch::channel(false).0,
            session: Mutex::new(None),
        });
        registry.openings.insert(id, state.clone());
        Opening {
            id,
            registry: self.0.clone(),
            state,
        }
    }

    pub fn find(&self, id: &str, context: &RemoteRequest) -> Option<Arc<Session>> {
        let mut registry = self.0.lock().expect("sessions lock");
        let session = registry.sessions.get(id)?;
        if !session.is_open() {
            registry.sessions.remove(id);
            return None;
        }
        session.matches(context).then(|| session.clone())
    }

    pub fn remove(&self, id: &str) {
        let session = self.0.lock().expect("sessions lock").sessions.remove(id);
        if let Some(session) = session {
            session.close("Legacy session deleted");
        }
    }

    pub fn forget(&self, plugin: &str) {
        let (sessions, openings) = {
            let mut registry = self.0.lock().expect("sessions lock");
            let sessions: Vec<_> = registry
                .sessions
                .extract_if(|_, session| session.plugin == plugin)
                .map(|(_, session)| session)
                .collect();
            let openings: Vec<_> = registry
                .openings
                .extract_if(|_, opening| opening.plugin == plugin)
                .map(|(_, opening)| opening)
                .collect();
            (sessions, openings)
        };
        for opening in openings {
            opening.cancelled.send_replace(true);
            let session = opening
                .session
                .lock()
                .expect("opening lock")
                .take()
                .and_then(|s| s.upgrade());
            if let Some(session) = session {
                session.close("Plugin was uninstalled during initialization");
            }
        }
        for session in sessions {
            session.close("Plugin was uninstalled");
        }
    }
}

impl Opening {
    pub(super) fn registry(&self) -> Weak<Mutex<Registry>> {
        Arc::downgrade(&self.registry)
    }

    pub fn cancellation(&self) -> watch::Receiver<bool> {
        self.state.cancelled.subscribe()
    }

    pub(super) fn attach(&self, session: &Arc<Session>) -> bool {
        let attached = {
            let registry = self.registry.lock().expect("sessions lock");
            if !registry.openings.contains_key(&self.id) || *self.state.cancelled.borrow() {
                false
            } else {
                *self.state.session.lock().expect("opening lock") = Some(Arc::downgrade(session));
                true
            }
        };
        if !attached {
            session.close("Plugin was uninstalled during initialization");
        }
        attached
    }

    pub(super) fn publish(&self) -> bool {
        let mut registry = self.registry.lock().expect("sessions lock");
        if !registry.openings.contains_key(&self.id) || *self.state.cancelled.borrow() {
            return false;
        }
        let mut attached = self.state.session.lock().expect("opening lock");
        let Some(session) = attached.as_ref().and_then(Weak::upgrade) else {
            return false;
        };
        if !session.is_open() {
            return false;
        }
        registry.sessions.insert(session.id.clone(), session);
        attached.take();
        true
    }
}

impl Drop for Opening {
    fn drop(&mut self) {
        self.registry
            .lock()
            .expect("sessions lock")
            .openings
            .remove(&self.id);
        let session = self
            .state
            .session
            .lock()
            .expect("opening lock")
            .take()
            .and_then(|s| s.upgrade());
        if let Some(session) = session {
            session.close("Legacy initialization was not completed");
        }
    }
}

pub(super) fn remove_closed(registry: &Weak<Mutex<Registry>>, id: &str) {
    if let Some(registry) = registry.upgrade() {
        registry.lock().expect("sessions lock").sessions.remove(id);
    }
}

pub(crate) async fn cancelled(mut receiver: watch::Receiver<bool>) {
    let _ = receiver.wait_for(|cancelled| *cancelled).await;
}
