//! Room panel state is keyed by Matrix room, never by the last opened window.

use super::*;

#[derive(Clone)]
pub(super) struct RoomPanelState {
    pub(super) panel: RoomPanel,
    pub(super) selected: Option<TabRef>,
    pub(super) blocked: bool,
    pub(super) removed: Vec<String>,
    pub(super) view: watch::Sender<Option<Arc<RoomPanelView>>>,
    session: std::sync::Weak<MatrixSession>,
}

impl RoomPanelState {
    pub(super) fn new(panel: RoomPanel) -> Self {
        Self {
            panel,
            selected: None,
            blocked: false,
            removed: Vec::new(),
            view: watch::Sender::new(None),
            session: std::sync::Weak::new(),
        }
    }
}

impl PagesLink {
    pub(crate) fn room_view(&self, room_id: &str) -> Option<Arc<RoomPanelView>> {
        let state = lock(&self.state);
        state.rooms.get(room_id)?.view.borrow().clone()
    }

    pub(crate) async fn subscribe_room_view(
        &self,
        room_id: &str,
    ) -> Result<watch::Receiver<Option<Arc<RoomPanelView>>>, CoreError> {
        let id: OwnedRoomId = room_id
            .parse()
            .map_err(|_| invalid(&l10n!(CATALOG, language(), "panel-room-unknown")))?;
        let mut session_changes = self.session.clone();
        let session =
            session_changes
                .borrow_and_update()
                .clone()
                .ok_or_else(|| CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "panel-not-signed-in"),
                })?;
        let changes = session.room_panel_changes(&id);
        let raw = session
            .room_panel(&id)
            .await
            .map_err(|error| crate::chat::core_error(&error))?;
        let panel = match raw {
            Some(raw) => {
                serde_json::from_value(raw.0.into()).map_err(|error| CoreError::InvalidInput {
                    reason: error.to_string(),
                })?
            }
            None => RoomPanel::default(),
        };
        let mut attached = None;
        self.runtime_change(|state, _| {
            if !session_changes
                .borrow()
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &session))
            {
                return Err(CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "panel-not-signed-in"),
                });
            }
            let fresh = state.rooms.get(room_id).is_none_or(|room| {
                !room
                    .session
                    .upgrade()
                    .is_some_and(|current| Arc::ptr_eq(&current, &session))
            });
            if fresh {
                let mut room = RoomPanelState::new(panel);
                room.session = Arc::downgrade(&session);
                state.rooms.insert(room_id.to_owned(), room);
            }
            let room = state.rooms.get_mut(room_id).expect("room state attached");
            let start = room.view.receiver_count() == 0;
            attached = Some((room.view.subscribe(), room.view.clone(), start));
            Ok(())
        })?;
        let (receiver, sender, start) = attached.expect("room attachment completed");
        if start {
            self.watch_room_panel(id, sender, session_changes, changes);
        }
        Ok(receiver)
    }

    fn watch_room_panel(
        &self,
        id: OwnedRoomId,
        sender: watch::Sender<Option<Arc<RoomPanelView>>>,
        mut session: watch::Receiver<Option<Arc<MatrixSession>>>,
        changes: impl futures_util::Stream<Item = wonton_matrix::RoomPanelContent> + Send + 'static,
    ) {
        let weak = lock(&self.weak).clone();
        self.runtime.spawn(async move {
            let mut changes = std::pin::pin!(changes);
            loop {
                tokio::select! {
                    () = sender.closed() => {
                        let Some(link) = weak.upgrade() else { break };
                        let mut removed = false;
                        let _ = link.runtime_change(|state, _| {
                            match state.rooms.get(id.as_str()) {
                                Some(room) if !room.view.same_channel(&sender) => removed = true,
                                Some(room) if room.view.receiver_count() > 0 => {}
                                _ => {
                                    state.rooms.remove(id.as_str());
                                    removed = true;
                                }
                            }
                            Ok(())
                        });
                        if removed { break; }
                    },
                    _ = session.changed() => break,
                    change = changes.next() => {
                        let Some(change) = change else { break };
                        let Some(link) = weak.upgrade() else { break };
                        match serde_json::from_value(change.0.into()) {
                            Ok(panel) => {
                                let _ = link.runtime_change(|state, _| {
                                    if let Some(room) = state.rooms.get_mut(id.as_str())
                                        && room.view.same_channel(&sender)
                                    {
                                        room.panel = panel;
                                    }
                                    Ok(())
                                });
                            }
                            Err(error) => { link.sync_problem.send_replace(Some(error.to_string())); }
                        }
                    }
                }
            }
            if let Some(link) = weak.upgrade() {
                let _ = link.runtime_change(|state, _| {
                    if state.rooms.get(id.as_str()).is_some_and(|room| room.view.same_channel(&sender)) {
                        state.rooms.remove(id.as_str());
                    }
                    Ok(())
                });
            }
        });
    }

    pub(super) fn receive_room_panel(
        &self,
        room_id: OwnedRoomId,
        panel: RoomPanel,
    ) -> Result<(), CoreError> {
        self.runtime_change(|state, _| {
            match state.rooms.entry(room_id.to_string()) {
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().panel = panel
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(RoomPanelState::new(panel));
                }
            }
            Ok(())
        })
    }

    pub(super) fn publish_room_views(&self, plugins: &PluginsViewModel) {
        let state = lock(&self.state);
        for (room_id, room) in &state.rooms {
            let scope = PanelScope::Room {
                room_id: room_id.clone(),
            };
            let view = RoomPanelView {
                panel: room_panel_items(&state, &room.panel, &scope, plugins),
                selected: room.selected.clone(),
                blocked: room.blocked,
            };
            room.view.send_if_modified(|current| {
                if current.as_deref() == Some(&view) {
                    return false;
                }
                *current = Some(Arc::new(view));
                true
            });
        }
    }

    pub(super) fn remove_uninstalled_room_pages(&self, state: &mut PagesState) {
        for (id, room) in &mut state.rooms {
            let removed = std::mem::take(&mut room.removed);
            if removed.is_empty() {
                continue;
            }
            let Some(session) = self.session.borrow().clone() else {
                continue;
            };
            let room_id: OwnedRoomId = id.parse().expect("registered Matrix room ID");
            let problem = self.sync_problem.clone();
            self.runtime.spawn(async move {
                let result = session
                    .update_room_panel(&room_id, |raw| {
                        let Ok(mut panel) =
                            serde_json::from_value::<RoomPanel>(std::mem::take(raw).into())
                        else {
                            return;
                        };
                        panel.pages.retain(|entry| match &entry.tab {
                            TabRef::Plugin { plugin_id, .. }
                            | TabRef::PluginPath { plugin_id, .. } => !removed.contains(plugin_id),
                            _ => true,
                        });
                        *raw = serde_json::to_value(panel)
                            .expect("serializable panel")
                            .as_object()
                            .expect("panel object")
                            .clone();
                    })
                    .await;
                if let Err(error) = result {
                    problem.send_replace(Some(error.to_string()));
                }
            });
        }
    }
}
