//! 每个打开的房间的视图（顶栏的 widget 入口，ADR-0096），按 Matrix 房间记，不按最后打开的窗口。

use super::*;

#[derive(Clone)]
pub(super) struct RoomViewState {
    view: watch::Sender<Option<Arc<RoomView>>>,
}

impl PagesLink {
    #[cfg(test)]
    pub(crate) fn room_view(&self, room_id: &str) -> Option<Arc<RoomView>> {
        let state = lock(&self.state);
        state.rooms.get(room_id)?.view.borrow().clone()
    }

    /// 时间线订阅打开房间时订阅它的视图；最后一个订阅放掉后这个房间的记录跟着清掉。
    pub(crate) fn subscribe_room_view(
        &self,
        room_id: &str,
    ) -> Result<watch::Receiver<Option<Arc<RoomView>>>, CoreError> {
        let _: OwnedRoomId = room_id
            .parse()
            .map_err(|_| invalid(&l10n!(CATALOG, language(), "chat-room-view-unknown")))?;
        let mut attached = None;
        self.runtime_change(|state, _| {
            let fresh = !state.rooms.contains_key(room_id);
            let room = state
                .rooms
                .entry(room_id.to_owned())
                .or_insert_with(|| RoomViewState {
                    view: watch::Sender::new(None),
                });
            attached = Some((room.view.subscribe(), room.view.clone(), fresh));
            Ok(())
        })?;
        let (receiver, sender, fresh) = attached.expect("room attachment completed");
        if fresh {
            self.forget_room_when_closed(room_id.to_owned(), sender);
        }
        Ok(receiver)
    }

    fn forget_room_when_closed(
        &self,
        room_id: String,
        sender: watch::Sender<Option<Arc<RoomView>>>,
    ) {
        let weak = lock(&self.weak).clone();
        self.runtime.spawn(async move {
            loop {
                sender.closed().await;
                let Some(link) = weak.upgrade() else { return };
                let mut done = false;
                let _ = link.runtime_change(|state, _| {
                    match state.rooms.get(&room_id) {
                        // 关掉的这一刻又有新订阅接上：接着等。
                        Some(room)
                            if room.view.same_channel(&sender)
                                && room.view.receiver_count() > 0 => {}
                        Some(room) if room.view.same_channel(&sender) => {
                            state.rooms.remove(&room_id);
                            done = true;
                        }
                        _ => done = true,
                    }
                    Ok(())
                });
                if done {
                    return;
                }
            }
        });
    }

    pub(super) fn publish_room_views(&self, plugins: &PluginsViewModel) {
        let state = lock(&self.state);
        for (room_id, room) in &state.rooms {
            let view = RoomView {
                room_id: room_id.parse().expect("registered Matrix room ID"),
                widgets: if self.plugins.is_external() {
                    Vec::new()
                } else {
                    widget::entries(room_id, plugins)
                },
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
}
