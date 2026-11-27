use wonton_protocol::{BotView, ChatContact, RoomRow};

use super::super::{contacts, lock};
use super::Bots;

impl Bots {
    /// Bot 列表和联系人候选在同一个视图写入里换：只看 `view.bots` 的壳不会撞上中间态。
    pub(in crate::chat) fn publish_bots_and_contacts(&self, local: &Local, bots: Vec<BotView>) {
        let contacts = {
            let view = self.view.borrow();
            contacts::assemble(
                &bots,
                local.shared.as_deref(),
                &view.rooms,
                view.own_private_bot.as_ref(),
            )
        };
        self.view.send_if_modified(|view| {
            let mut changed = false;
            if view.bots != bots {
                view.bots = bots;
                changed = true;
            }
            if view.shared_contacts != contacts {
                view.shared_contacts = contacts;
                changed = true;
            }
            changed
        });
    }

    /// 把当前能选的人合成进 ViewModel；壳只按搜索词过滤，不自己拼。
    pub(in crate::chat) fn publish_contacts(&self) {
        let contacts = {
            let rooms = self.view.borrow().rooms.clone();
            self.contacts_for(&rooms)
        };
        self.view.send_if_modified(|view| {
            if view.shared_contacts == contacts {
                return false;
            }
            view.shared_contacts = contacts;
            true
        });
    }

    /// 联系人按给定的房间行合成：调用方要把房间行和联系人放进同一次视图写入。
    fn contacts_for(&self, rooms: &[RoomRow]) -> Vec<ChatContact> {
        let local = lock(&self.local);
        let view = self.view.borrow();
        contacts::assemble(
            &view.bots,
            local.shared.as_deref(),
            rooms,
            view.own_private_bot.as_ref(),
        )
    }

    /// 房间行和联系人候选一起换：只看视图的壳不会撞上“房间已经认成旁聊、联系人还没跟上”的中间态。
    pub(in crate::chat) fn publish_rooms_and_contacts(&self, rooms: Vec<RoomRow>) {
        let contacts = self.contacts_for(&rooms);
        self.view.send_modify(|view| {
            view.rooms = rooms;
            if view.shared_contacts != contacts {
                view.shared_contacts = contacts;
            }
        });
    }
}
