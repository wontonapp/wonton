//! 房间列表接进 ViewModel：排序、把主人的 Bot 排前面、按合成好的显示方式填每一行。

use std::sync::Arc;

use futures_util::StreamExt;
use tokio::task::JoinHandle;
use wonton_matrix::MatrixSession;

use super::ChatLink;
use super::confirmation::core_account;
use super::layout::compose;
use super::lock;
use super::view::{room_row, sort_rooms};

impl ChatLink {
    /// 房间列表跟着 SDK 的房间流走：每批都重排、记进 ViewModel。显示方式变了也重算。
    pub(super) fn observe_rooms(self: &Arc<Self>, session: &Arc<MatrixSession>) -> JoinHandle<()> {
        let view = self.view.clone();
        let session = session.clone();
        let room_order = self.room_order.clone();
        let primary_bot = self.primary_bot.clone();
        let layouts = self.layouts.clone();
        let bots = self.bots.clone();
        let chat = Arc::downgrade(self);
        self.runtime.spawn(async move {
            let rooms = match session.rooms().await {
                Ok(rooms) => rooms,
                Err(error) => {
                    tracing::warn!("can't get the room list: {error}");
                    return;
                }
            };
            let core = core_account(session.user_id());
            let mut rooms = std::pin::pin!(rooms);
            let mut layouts = layouts.subscribe();
            // 列表流只给房间的变化，留着最近一批按新的选择重排。
            let mut latest = Vec::new();
            let mut ready = false;
            loop {
                tokio::select! {
                    batch = rooms.next() => {
                        let Some(batch) = batch else { return };
                        latest = batch;
                        ready = true;
                    }
                    Ok(()) = layouts.changed() => {}
                }
                if !ready {
                    continue;
                }
                let settings = layouts.borrow_and_update().clone();
                let mut rows: Vec<_> = latest
                    .iter()
                    .cloned()
                    .map(|room| {
                        let layout = match &room.direct_user_id {
                            Some(user) => compose(
                                &settings,
                                bots.chat_form(user.as_str())
                                    .map(|form| (user.as_str(), Some(form))),
                            ),
                            None => compose(&settings, None),
                        };
                        room_row(room, &core, layout)
                    })
                    .collect();
                sort_rooms(&mut rows);
                let bot = lock(&primary_bot).clone();
                let rows = room_order.replace(rows, bot.as_deref());
                view.send_modify(|view| view.rooms = rows);
                if let Some(chat) = chat.upgrade() {
                    chat.installs.refresh();
                }
            }
        })
    }
}
