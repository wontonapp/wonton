//! 对方核心发来的 [`ContactEvent`]（ADR-0050）。通道已经核对过发送者是 `sender_server` 的核心账号、那台已配对；
//! 这里核对 `from` 在发送方那台、`to` 在本机，对不上整条忽略。

use wonton_protocol::{
    ContactEvent, ContactKind, ContactOutcome, ContactRequestPush, ContactState,
};

use super::local::{active, incoming};
use super::store::Row;
use super::{Contacts, LocalBot, now_ms, server_of};

impl Contacts {
    /// 处理一条对方核心发来的事件。
    pub async fn receive(&self, sender_server: &str, event: ContactEvent) {
        let _op = self.ops.lock().await;
        let (from, to) = match &event {
            ContactEvent::Request { from, to, .. }
            | ContactEvent::Result { from, to, .. }
            | ContactEvent::Remove { from, to } => (from.clone(), to.clone()),
            ContactEvent::Other => return,
        };
        if server_of(&from) != Some(sender_server) || server_of(&to) != Some(&self.own_server) {
            tracing::warn!(%sender_server, %from, %to, "联系人事件的用户 ID 和发送方对不上，忽略");
            return;
        }
        let local = match self.ports.directory.bot_by_user(&to) {
            Ok(local) => local,
            Err(e) => {
                tracing::error!(error = %e, "读不出 Bot 的记录，联系人事件没处理");
                return;
            }
        };
        match event {
            ContactEvent::Request {
                id,
                from_display_name,
                keeper_display_name,
                ..
            } => {
                let from = Sender {
                    user_id: from,
                    display_name: from_display_name,
                    keeper_display_name,
                };
                self.received_request(sender_server, local, &to, id, from)
                    .await;
            }
            ContactEvent::Result {
                id,
                outcome,
                keeper_display_name,
                ..
            } => {
                if let Some(local) = local {
                    self.received_result(&local, &from, &id, outcome, keeper_display_name);
                }
            }
            ContactEvent::Remove { .. } => {
                if let Some(local) = local {
                    self.received_remove(&local, &from);
                }
            }
            ContactEvent::Other => {}
        }
    }

    async fn received_request(
        &self,
        sender_server: &str,
        local: Option<LocalBot>,
        to: &str,
        id: String,
        from: Sender,
    ) {
        let reply = |outcome, keeper: Option<String>| ContactEvent::Result {
            id: id.clone(),
            from: to.to_owned(),
            to: from.user_id.clone(),
            outcome,
            keeper_display_name: keeper,
        };
        let Some(local) = local else {
            self.reply(sender_server, reply(ContactOutcome::NotFound, None))
                .await;
            return;
        };
        let now = now_ms();
        let existing = self.lock().row(&local.bot, &from.user_id).cloned();
        let mutual = existing.as_ref().is_some_and(|row| {
            row.kind == ContactKind::Bot
                && matches!(row.state, ContactState::Pending | ContactState::Active)
        });
        if mutual {
            // 这边也请求过对方（两边同时发），或者早就成立了：直接成立并回同意。
            let row = existing.expect("刚看过有这一行");
            let was_active = row.state == ContactState::Active;
            let row = Row {
                display_name: from.display_name.clone().or(row.display_name.clone()),
                keeper_display_name: from.keeper_display_name.clone(),
                ..active(row, now)
            };
            if self
                .log_commit(|stored| stored.put(&local.bot, &from.user_id, row))
                .is_ok()
                && !was_active
            {
                self.ports.effects.allowed_users_changed(&local.bot);
            }
            let keeper = local.keeper_display_name.clone();
            self.reply(sender_server, reply(ContactOutcome::Approved, keeper))
                .await;
            return;
        }
        let fresh = existing
            .as_ref()
            .is_none_or(|row| row.state != ContactState::Incoming);
        let row = Row {
            display_name: from.display_name.clone(),
            keeper_display_name: from.keeper_display_name.clone(),
            ..incoming(&local, Some(id.clone()), now)
        };
        if self
            .log_commit(|stored| stored.put(&local.bot, &from.user_id, row))
            .is_ok()
            && fresh
        {
            self.ports.effects.notify_request(
                &local.keeper_username,
                ContactRequestPush {
                    bot: local.bot.clone(),
                    user_id: from.user_id,
                    display_name: from.display_name,
                    keeper_display_name: from.keeper_display_name,
                },
            );
        }
    }

    fn received_result(
        &self,
        local: &LocalBot,
        from: &str,
        id: &str,
        outcome: ContactOutcome,
        keeper_display_name: Option<String>,
    ) {
        let next = match outcome {
            ContactOutcome::Approved => ContactState::Active,
            ContactOutcome::Rejected => ContactState::Declined,
            ContactOutcome::NotFound => ContactState::NotFound,
            ContactOutcome::Other => return,
        };
        let Some(mut row) = self.lock().row(&local.bot, from).cloned() else {
            return;
        };
        // 删了再请求时，旧请求迟到的结果对不上新的 ID，不算数。
        if row.kind != ContactKind::Bot
            || row.state != ContactState::Pending
            || row.request_id.as_deref() != Some(id)
        {
            tracing::info!(bot = %local.user_id, contact = %from, "联系人结果对不上还在等的请求，忽略");
            return;
        }
        row.state = next;
        row.updated_at_ms = now_ms();
        if keeper_display_name.is_some() {
            row.keeper_display_name = keeper_display_name;
        }
        if self
            .log_commit(|stored| stored.put(&local.bot, from, row))
            .is_ok()
            && next == ContactState::Active
        {
            self.ports.effects.allowed_users_changed(&local.bot);
        }
    }

    fn received_remove(&self, local: &LocalBot, from: &str) {
        let Some(row) = self.lock().row(&local.bot, from).cloned() else {
            return;
        };
        if row.kind != ContactKind::Bot {
            return;
        }
        if self
            .log_commit(|stored| {
                stored.take(&local.bot, from);
            })
            .is_ok()
            && row.state == ContactState::Active
        {
            self.ports.effects.allowed_users_changed(&local.bot);
        }
    }

    /// 收事件这条路上没有人等着看错误：写不进去记日志。
    fn log_commit(
        &self,
        change: impl FnOnce(&mut super::Stored),
    ) -> Result<(), super::ContactFailure> {
        self.commit(change)
            .inspect_err(|e| tracing::error!(error = %e, "联系人记录写不进去"))
    }

    /// 回对方一个结果。发不出去时对方一直是“等对方同意”，记日志。
    async fn reply(&self, server: &str, event: ContactEvent) {
        if let Err(e) = self.ports.link.send(server, event).await {
            tracing::warn!(%server, error = %e, "联系人结果没能发给对方");
        }
    }
}

struct Sender {
    user_id: String,
    display_name: Option<String>,
    keeper_display_name: Option<String>,
}
