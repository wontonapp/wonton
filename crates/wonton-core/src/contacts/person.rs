//! 对方家里的人做联系人（ADR-0050）：Bot 发带标记的私聊邀请，他接受才成立。

use wonton_protocol::{ContactKind, ContactState};

use super::store::Row;
use super::{ContactFailure, Contacts, InviteOutcome, InviteeState, LocalBot, now_ms};

impl Contacts {
    /// 发邀请并记下结果。对方服务器连不上、Tuwunel 出错时什么都不记。
    pub(super) async fn request_person(
        &self,
        local: &LocalBot,
        user: &str,
    ) -> Result<(), ContactFailure> {
        let outcome = self.ports.invites.invite(local, user).await?;
        let (state, room_id) = match outcome {
            InviteOutcome::Sent { room_id } => (ContactState::Pending, Some(room_id)),
            InviteOutcome::Declined => (ContactState::Declined, None),
            InviteOutcome::NotFound => (ContactState::NotFound, None),
        };
        let now = now_ms();
        self.commit(|stored| {
            stored.put(
                &local.bot,
                user,
                Row {
                    kind: ContactKind::Person,
                    state,
                    display_name: None,
                    keeper_display_name: None,
                    request_id: None,
                    room_id,
                    updated_at_ms: now,
                },
            );
        })
    }

    /// 删一个人：还在等他接受时撤回邀请，撤不回就不删（不然他之后接受了，这边却没有这一行）。
    pub(super) async fn remove_person(
        &self,
        local: &LocalBot,
        user: &str,
        row: &Row,
    ) -> Result<(), ContactFailure> {
        if row.state == ContactState::Pending
            && let Some(room) = &row.room_id
        {
            self.ports.invites.withdraw(local, room, user).await?;
        }
        self.commit(|stored| {
            stored.take(&local.bot, user);
        })
    }

    /// 查一遍所有 Bot 还在等的人：接受了就成立，拒绝了就是“对方拒绝了”。后台定时调。
    pub async fn check_people(&self) {
        let bots: Vec<_> = {
            let stored = self.lock();
            let mut bots: Vec<_> = stored
                .all()
                .filter(|(_, _, row)| waiting_person(row))
                .map(|(bot, _, _)| bot.clone())
                .collect();
            bots.dedup();
            bots
        };
        for bot in bots {
            match self.ports.directory.bot(&bot) {
                Ok(Some(local)) => self.check_people_of(&local).await,
                Ok(None) => {}
                Err(e) => tracing::warn!(bot = %bot.bot, error = %e, "读不出 Bot 的记录"),
            }
        }
    }

    pub(super) async fn check_people_of(&self, local: &LocalBot) {
        let waiting: Vec<(String, String)> = self
            .lock()
            .rows(&local.bot)
            .into_iter()
            .flatten()
            .filter(|(_, row)| waiting_person(row))
            .filter_map(|(user, row)| Some((user.clone(), row.room_id.clone()?)))
            .collect();
        for (user, room) in waiting {
            let state = match self.ports.invites.state(local, &room, &user).await {
                Ok(state) => state,
                Err(e) => {
                    tracing::info!(bot = %local.user_id, contact = %user, error = %e, "查不到邀请的结果，下次再查");
                    continue;
                }
            };
            let next = match state {
                InviteeState::Invited => continue,
                InviteeState::Joined => ContactState::Active,
                InviteeState::Left => ContactState::Declined,
            };
            let _op = self.ops.lock().await;
            let now = now_ms();
            let changed = self.commit(|stored| {
                if let Some(mut row) = stored.row(&local.bot, &user).cloned()
                    && waiting_person(&row)
                    && row.room_id.as_deref() == Some(room.as_str())
                {
                    row.state = next;
                    row.updated_at_ms = now;
                    stored.put(&local.bot, &user, row);
                }
            });
            match changed {
                Ok(()) if next == ContactState::Active => {
                    tracing::info!(bot = %local.user_id, contact = %user, "对方接受了联系人邀请");
                    self.ports.effects.allowed_users_changed(&local.bot);
                }
                Ok(()) => {}
                Err(e) => tracing::error!(error = %e, "联系人记录写不进去"),
            }
        }
    }
}

fn waiting_person(row: &Row) -> bool {
    row.kind == ContactKind::Person && row.state == ContactState::Pending
}
