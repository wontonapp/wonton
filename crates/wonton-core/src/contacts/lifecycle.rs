//! 联系人的生命周期：批、拒、删。每次改动都在一个事务里落盘（行和给对方的出站事件一起），成了才进
//! 内存；事件发不出去留在 outbox 里，收件循环按退避重试（ADR-0050“先落意图再发”）。

use std::sync::Arc;

use wonton_protocol::{
    BotContacts, BotRef, ContactError, ContactEvent, ContactKind, ContactOutcome, ContactState,
};

use super::{Contacts, exchange};
use crate::peers;

impl Contacts {
    /// 归属人同意一条进来的请求。同机的请求一次成立（发起方那行也落成 Active）；跨机的把结果
    /// 事件入队发送，发送失败不改变本地的同意。
    pub async fn approve(
        self: &Arc<Self>,
        bot: &BotRef,
        user_id: &str,
    ) -> Result<BotContacts, ContactError> {
        let at = peers::now_ms();
        let bots = self.bots.get().ok_or(ContactError::Upstream)?;
        let bot_user = bots.user_of_bot(bot).ok_or(ContactError::BotNotFound)?;
        // 同机的发起方 Bot 在事务外解析好，闭包里只动表。
        let requester = bots.bot_of_user(user_id);
        let keeper_display = bots.keeper_display_name(bot);

        let server = self.commit(|stored| -> Result<Option<String>, ContactError> {
            let row = stored
                .approve(bot, user_id, at)
                .map_err(|()| ContactError::ContactNotFound)?;
            // 同机的请求一次成立：发起方那行（Pending）也一起落成 Active。
            if row.remote.is_none()
                && let Some(counterpart) = &requester
                && let Some(mut pending) = stored.row(counterpart, &bot_user)
                && pending.state == ContactState::Pending
            {
                pending.state = ContactState::Active;
                pending.updated_at_ms = at;
                stored.upsert(counterpart, pending);
            }
            // 跨机的：同意的结果事件和行一起落盘，送不送得到由收件循环管。
            match (&row.remote, &row.request_id) {
                (Some(server), Some(id)) => {
                    stored.enqueue(
                        server,
                        ContactEvent::Result {
                            id: id.clone(),
                            from: bot_user.clone(),
                            to: user_id.to_owned(),
                            outcome: ContactOutcome::Approved,
                            keeper_display_name: keeper_display.clone(),
                        },
                        id.clone(),
                        at,
                    );
                    Ok(Some(server.clone()))
                }
                _ => Ok(None),
            }
        })??;

        self.rebuild(bot);
        if let Some(requester) = &requester {
            self.rebuild(requester);
        }
        if let Some(server) = server {
            exchange::drain(self, &server).await;
        }
        Ok(self.view(bot))
    }

    /// 归属人拒绝一条进来的请求：这一行随即消失（ADR-0050）。同机的请求把发起方那行落成“对方拒绝了”
    /// （同一个事务）；跨机的把拒绝事件入队，由对方落。
    pub async fn reject(
        self: &Arc<Self>,
        bot: &BotRef,
        user_id: &str,
    ) -> Result<BotContacts, ContactError> {
        let at = peers::now_ms();
        let bots = self.bots.get().ok_or(ContactError::Upstream)?;
        let bot_user = bots.user_of_bot(bot).ok_or(ContactError::BotNotFound)?;
        let keeper_display = bots.keeper_display_name(bot);
        // 同机的发起方 Bot：它的 Pending 行在同一个事务里落成 Declined。
        let requester = bots.bot_of_user(user_id).filter(|other| *other != *bot);

        let server = self.commit(|stored| -> Result<Option<String>, ContactError> {
            let row = stored
                .row(bot, user_id)
                .ok_or(ContactError::ContactNotFound)?;
            if row.state != ContactState::Incoming {
                return Err(ContactError::ContactNotFound);
            }
            stored.remove(bot, user_id);
            if row.remote.is_none()
                && let Some(counterpart) = &requester
                && let Some(mut pending) = stored.row(counterpart, &bot_user)
                && pending.state == ContactState::Pending
            {
                pending.state = ContactState::Declined;
                pending.updated_at_ms = at;
                stored.upsert(counterpart, pending);
            }
            match (&row.remote, &row.request_id) {
                (Some(server), Some(id)) => {
                    stored.enqueue(
                        server,
                        ContactEvent::Result {
                            id: id.clone(),
                            from: bot_user.clone(),
                            to: user_id.to_owned(),
                            outcome: ContactOutcome::Rejected,
                            keeper_display_name: keeper_display.clone(),
                        },
                        id.clone(),
                        at,
                    );
                    Ok(Some(server.clone()))
                }
                _ => Ok(None),
            }
        })??;

        if let Some(server) = server {
            exchange::drain(self, &server).await;
        }
        Ok(self.view(bot))
    }

    /// 归属人删掉联系人：也用来取消自己发出的请求、清掉“对方拒绝了”“没有这个人”的行（协议 `bot_contact_path`）。
    /// 删一条进来的请求按拒绝算（ADR-0050），远端的也一样：对面发起方看到“对方拒绝了”，不是两边都凭空消失。
    pub async fn delete(
        self: &Arc<Self>,
        bot: &BotRef,
        user_id: &str,
    ) -> Result<BotContacts, ContactError> {
        if self
            .lock()
            .row(bot, user_id)
            .is_some_and(|row| row.state == ContactState::Incoming)
        {
            return self.reject(bot, user_id).await;
        }
        let at = peers::now_ms();
        let bots = self.bots.get().ok_or(ContactError::Upstream)?;
        let bot_user = bots.user_of_bot(bot).ok_or(ContactError::BotNotFound)?;
        let row = self
            .lock()
            .row(bot, user_id)
            .ok_or(ContactError::ContactNotFound)?;
        // 同机：这层关系是两行，对面那行在同一个事务里删掉。
        let counterpart = if row.remote.is_none() {
            bots.bot_of_user(user_id)
                .filter(|other| *other != *bot)
                .map(|other| {
                    let was_active = self
                        .lock()
                        .row(&other, &bot_user)
                        .is_some_and(|row| row.state == ContactState::Active);
                    (other, was_active)
                })
        } else {
            None
        };

        let remote_server = self.commit(|stored| -> Result<Option<String>, ContactError> {
            stored
                .remove(bot, user_id)
                .ok_or(ContactError::ContactNotFound)?;
            if let Some((counterpart, _)) = &counterpart {
                stored.remove(counterpart, &bot_user);
            }
            // 跨机的 Bot：删除事件和行一起落盘；远端的人还没接受时下面单独撤回邀请。
            match (&row.remote, row.kind) {
                (Some(server), ContactKind::Bot) => {
                    stored.enqueue(
                        server,
                        ContactEvent::Remove {
                            from: bot_user.clone(),
                            to: user_id.to_owned(),
                        },
                        crate::secret::random_token(),
                        at,
                    );
                    Ok(Some(server.clone()))
                }
                _ => Ok(None),
            }
        })??;

        if row.state == ContactState::Active {
            self.rebuild(bot);
        }
        if let Some((counterpart, was_active)) = &counterpart
            && *was_active
        {
            self.rebuild(counterpart);
        }
        // 远端的人还没接受：撤回邀请（协议 ContactState::Pending 的“对人是撤回邀请”）。
        if row.remote.is_some()
            && row.kind == ContactKind::Person
            && row.state == ContactState::Pending
            && let Some(room) = &row.room
        {
            exchange::revoke_person_invite(self, bot, room, user_id).await;
        }
        if let Some(server) = remote_server {
            exchange::drain(self, &server).await;
        }
        Ok(self.view(bot))
    }
}
