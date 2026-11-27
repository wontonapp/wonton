//! 房间列表接进 ViewModel：排序、把主人的 Bot 排前面、按合成好的显示方式填每一行。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures_util::StreamExt;
use tokio::task::JoinHandle;
use wonton_matrix::MatrixSession;
use wonton_protocol::BotChatAvailability;

use super::ChatLink;
use super::core_account;
use super::layout::compose;
use super::lock;
use super::view::{room_row, sort_rooms};

impl ChatLink {
    /// 房间列表跟着 SDK 的房间流走：每批都重排、记进 ViewModel。显示方式变了也重算。
    pub(super) fn observe_rooms(self: &Arc<Self>, session: &Arc<MatrixSession>) -> JoinHandle<()> {
        let view = self.view.clone();
        let session = session.clone();
        let room_order = self.room_order.clone();
        let default_bot = self.default_bot.clone();
        let layouts = self.layouts.clone();
        let bots = self.bots.clone();
        let support = self.side_chat_support.clone();
        let runtime = self.runtime.clone();
        let chat = Arc::downgrade(self);
        let plugins = self.plugins.subscribe();
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
            let mut declarations = support.subscribe();
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
                    Ok(()) = declarations.changed() => {}
                    // Bot 的资格变了（授权、角色、断线）：已打开的行跟着换原因。
                    () = bots.changed.notified() => {}
                }
                if !ready {
                    continue;
                }
                let settings = layouts.borrow_and_update().clone();
                let availability: HashMap<String, BotChatAvailability> = view
                    .borrow()
                    .bots
                    .iter()
                    .map(|bot| (bot.user_id.clone(), bot.availability))
                    .collect();
                let bot_users: HashSet<String> = availability.keys().cloned().collect();
                // 和 Bot 的私聊取一次旁聊的支持声明（ADR-0093 §1）；进房间时再取。
                for room in &latest {
                    if room
                        .direct_user_id
                        .as_ref()
                        .is_some_and(|user| bot_users.contains(user.as_str()))
                    {
                        support.ensure(&runtime, &session, &room.room_id);
                    }
                }
                let chat = chat.upgrade();
                let mut rows: Vec<_> = latest
                    .iter()
                    .cloned()
                    .map(|room| {
                        let bot = if room.is_direct {
                            chat.as_ref().and_then(|chat| {
                                chat.bots.room_bot(
                                    room.direct_user_id.as_ref().map(|user| user.as_str()),
                                    &room.name,
                                )
                            })
                        } else {
                            None
                        };
                        let layout = match &room.direct_user_id {
                            Some(user) => compose(
                                &settings,
                                bots.chat_form(user.as_str())
                                    .map(|form| (user.as_str(), Some(form))),
                            ),
                            None => compose(&settings, None),
                        };
                        let side_chats = support.supports(&room.room_id, &bot_users);
                        room_row(room, &core, layout, &availability, bot, side_chats)
                    })
                    .collect();
                super::plugin_status::refresh(&mut rows, &plugins.borrow());
                sort_rooms(&mut rows);
                let bot = lock(&default_bot).clone();
                let rows = room_order.replace(rows, bot.as_deref());
                view.send_modify(|view| view.rooms = rows);
                if let Some(chat) = chat {
                    chat.installs.refresh();
                    chat.bots.publish_contacts();
                }
            }
        })
    }
}
