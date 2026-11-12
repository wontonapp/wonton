//! 归属人在 App 里的操作：发请求、同意、拒绝、删除。

use wonton_protocol::{
    BotContact, BotRef, ContactError, ContactEvent, ContactKind, ContactOutcome, ContactRequest,
    ContactRequestPush, ContactState, PeerStatus,
};

use super::store::Row;
use super::{ContactFailure, Contacts, LocalBot, PeerReach, now_ms, server_of};

impl Contacts {
    fn is_local(&self, user_id: &str) -> bool {
        server_of(user_id) == Some(self.own_server.as_str())
    }

    /// 请对方做这个 Bot 的联系人（[`ContactRequest`]）。
    pub async fn request(
        &self,
        bot: &BotRef,
        username: &str,
        request: &ContactRequest,
    ) -> Result<Vec<BotContact>, ContactFailure> {
        let _op = self.ops.lock().await;
        let local = self.keeper_bot(bot, username)?;
        let user = request.user_id.as_str();
        let server = server_of(user).ok_or_else(|| {
            ContactFailure::new(
                ContactError::InvalidUserId,
                format!("{user} 不是完整的 Matrix 用户 ID"),
            )
        })?;
        if user == local.user_id || request.kind == ContactKind::Other {
            return Err(ContactFailure::new(
                ContactError::InvalidUserId,
                format!("不能把 {user} 加成 {} 的联系人", local.user_id),
            ));
        }
        if server != self.own_server {
            check_reach(server, self.ports.link.reach(server))?;
        }
        let existing = self.lock().row(bot, user).cloned();
        match existing.as_ref().map(|row| row.state) {
            Some(ContactState::Active | ContactState::Pending) => {
                return Err(ContactFailure::new(
                    ContactError::AlreadyContact,
                    format!("{user} 已经是联系人，或者请求已经发出"),
                ));
            }
            // 对方先请求了这边：再请求就是同意（两边都同意了）。
            Some(ContactState::Incoming) if request.kind == ContactKind::Bot => {
                self.approve_row(&local, user, existing.expect("刚看过有这一行"))
                    .await?;
                return Ok(self.view(bot));
            }
            Some(ContactState::Incoming) => {
                return Err(ContactFailure::new(
                    ContactError::AlreadyContact,
                    format!("{user} 是 Bot，已经请求过这个 Bot"),
                ));
            }
            _ => {}
        }
        match request.kind {
            ContactKind::Person => self.request_person(&local, user).await?,
            _ if self.is_local(user) => self.request_local_bot(&local, user)?,
            _ => self.request_remote_bot(&local, user, server).await?,
        }
        Ok(self.view(bot))
    }

    fn request_local_bot(&self, local: &LocalBot, user: &str) -> Result<(), ContactFailure> {
        let target = self
            .ports
            .directory
            .bot_by_user(user)
            .map_err(|e| ContactFailure::new(ContactError::Upstream, e))?;
        let now = now_ms();
        let Some(target) = target else {
            return self.commit(|stored| {
                stored.put(
                    &local.bot,
                    user,
                    outgoing(ContactState::NotFound, None, now),
                );
            });
        };
        self.commit(|stored| {
            stored.put(
                &local.bot,
                user,
                Row {
                    display_name: target.display_name.clone(),
                    keeper_display_name: target.keeper_display_name.clone(),
                    ..outgoing(ContactState::Pending, None, now)
                },
            );
            stored.put(&target.bot, &local.user_id, incoming(local, None, now));
        })?;
        self.ports
            .effects
            .notify_request(&target.keeper_username, request_push(&target.bot, local));
        Ok(())
    }

    async fn request_remote_bot(
        &self,
        local: &LocalBot,
        user: &str,
        server: &str,
    ) -> Result<(), ContactFailure> {
        let id = crate::secret::random_token();
        let event = ContactEvent::Request {
            id: id.clone(),
            from: local.user_id.clone(),
            to: user.to_owned(),
            from_display_name: local.display_name.clone(),
            keeper_display_name: local.keeper_display_name.clone(),
        };
        self.ports.link.send(server, event).await?;
        let now = now_ms();
        self.commit(|stored| {
            stored.put(
                &local.bot,
                user,
                outgoing(ContactState::Pending, Some(id), now),
            );
        })
    }

    /// 同意对方发来的请求。
    pub async fn approve(
        &self,
        bot: &BotRef,
        username: &str,
        user: &str,
    ) -> Result<Vec<BotContact>, ContactFailure> {
        let _op = self.ops.lock().await;
        let local = self.keeper_bot(bot, username)?;
        let row = self.incoming_row(bot, user)?;
        self.approve_row(&local, user, row).await?;
        Ok(self.view(bot))
    }

    fn incoming_row(&self, bot: &BotRef, user: &str) -> Result<Row, ContactFailure> {
        self.lock()
            .row(bot, user)
            .filter(|row| row.state == ContactState::Incoming)
            .cloned()
            .ok_or_else(|| not_found(user))
    }

    async fn approve_row(
        &self,
        local: &LocalBot,
        user: &str,
        row: Row,
    ) -> Result<(), ContactFailure> {
        let now = now_ms();
        if self.is_local(user) {
            let Some(other) = self.other_local(user)? else {
                // 发请求的 Bot 已经不在了：这条请求也就不在了。
                self.commit(|stored| {
                    stored.take(&local.bot, user);
                })?;
                return Err(not_found(user));
            };
            self.commit(|stored| {
                stored.put(&local.bot, user, active(row, now));
                if let Some(theirs) = stored.row(&other.bot, &local.user_id).cloned() {
                    stored.put(&other.bot, &local.user_id, active(theirs, now));
                }
            })?;
            self.ports.effects.allowed_users_changed(&other.bot);
        } else {
            let server = server_of(user).unwrap_or_default();
            let event = ContactEvent::Result {
                id: row.request_id.clone().unwrap_or_default(),
                from: local.user_id.clone(),
                to: user.to_owned(),
                outcome: ContactOutcome::Approved,
                keeper_display_name: local.keeper_display_name.clone(),
            };
            self.ports.link.send(server, event).await?;
            self.commit(|stored| stored.put(&local.bot, user, active(row, now)))?;
        }
        self.ports.effects.allowed_users_changed(&local.bot);
        Ok(())
    }

    fn other_local(&self, user: &str) -> Result<Option<LocalBot>, ContactFailure> {
        self.ports
            .directory
            .bot_by_user(user)
            .map_err(|e| ContactFailure::new(ContactError::Upstream, e))
    }

    /// 拒绝对方发来的请求：这一行消失，对方看到“对方拒绝了”。
    pub async fn reject(
        &self,
        bot: &BotRef,
        username: &str,
        user: &str,
    ) -> Result<Vec<BotContact>, ContactFailure> {
        let _op = self.ops.lock().await;
        let local = self.keeper_bot(bot, username)?;
        let row = self.incoming_row(bot, user)?;
        let now = now_ms();
        if self.is_local(user) {
            let other = self.other_local(user)?;
            self.commit(|stored| {
                stored.take(&local.bot, user);
                if let Some(other) = &other
                    && let Some(mut theirs) = stored.row(&other.bot, &local.user_id).cloned()
                    && theirs.state == ContactState::Pending
                {
                    theirs.state = ContactState::Declined;
                    theirs.updated_at_ms = now;
                    stored.put(&other.bot, &local.user_id, theirs);
                }
            })?;
        } else {
            let event = ContactEvent::Result {
                id: row.request_id.unwrap_or_default(),
                from: local.user_id.clone(),
                to: user.to_owned(),
                outcome: ContactOutcome::Rejected,
                keeper_display_name: local.keeper_display_name.clone(),
            };
            self.ports
                .link
                .send(server_of(user).unwrap_or_default(), event)
                .await?;
            self.commit(|stored| {
                stored.take(&local.bot, user);
            })?;
        }
        Ok(self.view(bot))
    }

    /// 删掉联系人、取消请求，或清掉“对方拒绝了”“没有这个人”的行。
    pub async fn remove(
        &self,
        bot: &BotRef,
        username: &str,
        user: &str,
    ) -> Result<Vec<BotContact>, ContactFailure> {
        let _op = self.ops.lock().await;
        let local = self.keeper_bot(bot, username)?;
        let row = self
            .lock()
            .row(bot, user)
            .cloned()
            .ok_or_else(|| not_found(user))?;
        let was_active = row.state == ContactState::Active;
        let open = matches!(
            row.state,
            ContactState::Pending | ContactState::Incoming | ContactState::Active
        );
        match row.kind {
            ContactKind::Person => self.remove_person(&local, user, &row).await?,
            _ if self.is_local(user) => {
                let other = self.other_local(user)?;
                self.commit(|stored| {
                    stored.take(&local.bot, user);
                    if let Some(other) = &other {
                        stored.take(&other.bot, &local.user_id);
                    }
                })?;
                if let Some(other) = other
                    && was_active
                {
                    self.ports.effects.allowed_users_changed(&other.bot);
                }
            }
            _ => {
                self.commit(|stored| {
                    stored.take(&local.bot, user);
                })?;
                if open {
                    self.tell_removed(&local, user).await;
                }
            }
        }
        if was_active {
            self.ports.effects.allowed_users_changed(&local.bot);
        }
        Ok(self.view(bot))
    }

    /// 告诉对方删了。发不出去只记日志：这边已经删了，对方的话 Bot 不再处理；对方那一行留着，直到对方自己删。
    async fn tell_removed(&self, local: &LocalBot, user: &str) {
        let event = ContactEvent::Remove {
            from: local.user_id.clone(),
            to: user.to_owned(),
        };
        if let Err(e) = self
            .ports
            .link
            .send(server_of(user).unwrap_or_default(), event)
            .await
        {
            tracing::warn!(bot = %local.user_id, contact = %user, error = %e, "删除联系人没能通知对方");
        }
    }
}

/// 新请求要对方服务器在线：探活说连不上时直接失败，不记一个发不过去的请求（ADR-0050）。还在确认时照常发，
/// 发不过去由通道报错。
fn check_reach(server: &str, reach: PeerReach) -> Result<(), ContactFailure> {
    match reach {
        PeerReach::NotPaired | PeerReach::Paired(PeerStatus::RemovedByPeer) => {
            Err(ContactFailure::new(
                ContactError::NotPaired,
                format!("服务器 {server} 没有和这台配对"),
            ))
        }
        PeerReach::Paired(PeerStatus::Offline) => Err(ContactFailure::new(
            ContactError::PeerUnreachable,
            format!("服务器 {server} 现在连不上"),
        )),
        PeerReach::Paired(_) => Ok(()),
    }
}

pub(super) fn not_found(user: &str) -> ContactFailure {
    ContactFailure::new(
        ContactError::ContactNotFound,
        format!("没有和 {user} 的这一行"),
    )
}

pub(super) fn outgoing(state: ContactState, request_id: Option<String>, now: u64) -> Row {
    Row {
        kind: ContactKind::Bot,
        state,
        display_name: None,
        keeper_display_name: None,
        request_id,
        room_id: None,
        updated_at_ms: now,
    }
}

/// `from` 发来的请求在收到的那一方的样子。
pub(super) fn incoming(from: &LocalBot, request_id: Option<String>, now: u64) -> Row {
    Row {
        kind: ContactKind::Bot,
        state: ContactState::Incoming,
        display_name: from.display_name.clone(),
        keeper_display_name: from.keeper_display_name.clone(),
        request_id,
        room_id: None,
        updated_at_ms: now,
    }
}

pub(super) fn active(mut row: Row, now: u64) -> Row {
    row.state = ContactState::Active;
    row.updated_at_ms = now;
    row
}

pub(super) fn request_push(bot: &BotRef, from: &LocalBot) -> ContactRequestPush {
    ContactRequestPush {
        bot: bot.clone(),
        user_id: from.user_id.clone(),
        display_name: from.display_name.clone(),
        keeper_display_name: from.keeper_display_name.clone(),
    }
}
