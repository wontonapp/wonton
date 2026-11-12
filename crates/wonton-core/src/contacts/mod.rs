//! Bot 的联系人（ADR-0045），包括别的服务器上的 Bot 和人（ADR-0050；有名字模式下加“人”不要对方配对，
//! 见 [`Ports::open_federation`]）。
//!
//! - 记录在 `core/contacts.json`（[`store`]），请求不过期，重启后还在。每个 Bot 一张表，对方的用户 ID 一行。
//! - 同机的 Bot 之间直接改两张表。跨机的 Bot 经 [`PeerLink`] 和对方核心交换 [`ContactEvent`]；对方连不上时
//!   请求失败、什么都不记（[`ContactError::PeerUnreachable`]）。删除的通知和收到请求后回的结果不能这样丢，
//!   排进 [`outbox`] 直到送到。
//! - 对人：Bot 发带标记的私聊邀请（[`PersonInvites`]），他接受就成立。接受、拒绝都只能从房间里看出来，所以
//!   列表时和后台定时查一遍还在等的邀请（[`Contacts::check_people`]）。
//! - 名单变了调 [`Effects`]：重建 Bot 的服务（`WONTON_MATRIX_ALLOWED_USERS`），新请求推送给归属人。
//!
//! 所有改记录的操作排成一队（`ops`）：发事件、发邀请要等网络，等的时候别的操作不能插进来改同一行。

mod local;
mod matrix_invites;
mod matrix_link;
mod outbox;
mod person;
pub mod ports;
mod remote;
mod store;
pub mod wiring;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::watch;
use wonton_protocol::{
    BotContact, BotRef, ContactError, ContactState, PendingContactRequest, RemoteContact,
};

pub use ports::{
    ContactFailure, Directory, Effects, InviteOutcome, InviteeState, LocalBot, PeerLink, PeerReach,
    PersonInvites,
};
pub use store::StoreError;
use store::{Row, Stored};

/// 联系人逻辑用到的外部接口。
pub struct Ports {
    pub directory: Arc<dyn Directory>,
    pub link: Arc<dyn PeerLink>,
    pub invites: Arc<dyn PersonInvites>,
    pub effects: Arc<dyn Effects>,
    /// 有名字模式（ADR-0051）：Tuwunel 对整个 Matrix 开放联邦，加“人”不要求对方配对；
    /// 私用名保留 ADR-0050 的配对要求。
    pub open_federation: bool,
}

pub struct Contacts {
    path: PathBuf,
    own_server: String,
    stored: Mutex<Stored>,
    ops: tokio::sync::Mutex<()>,
    ports: Ports,
    /// 记录每变一次加一。
    changes: watch::Sender<u64>,
}

impl std::fmt::Debug for Contacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Contacts")
            .field("path", &self.path)
            .field("own_server", &self.own_server)
            .finish_non_exhaustive()
    }
}

/// `@名字:服务器名` 的服务器名；不是完整的 Matrix 用户 ID 时为空。
pub(crate) fn server_of(user_id: &str) -> Option<&str> {
    let rest = user_id.strip_prefix('@')?;
    let (local, server) = rest.split_once(':')?;
    (!local.is_empty() && !server.is_empty() && !server.contains('/')).then_some(server)
}

impl Contacts {
    /// 读记录；文件不在时从空表开始，损坏时报错（不当成没有联系人）。
    pub fn open(
        path: impl Into<PathBuf>,
        own_server: &str,
        ports: Ports,
    ) -> Result<Arc<Self>, StoreError> {
        let path = path.into();
        let stored = store::read(&path)?;
        Ok(Arc::new(Self {
            path,
            own_server: own_server.to_owned(),
            stored: Mutex::new(stored),
            ops: tokio::sync::Mutex::new(()),
            ports,
            changes: watch::channel(0).0,
        }))
    }

    pub fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// 记录变过多少次：“等你确认”列表把它算进版本，联系人请求有变化就唤醒挂着的长轮询。
    pub fn changes_count(&self) -> u64 {
        *self.changes.borrow()
    }

    fn lock(&self) -> MutexGuard<'_, Stored> {
        self.stored.lock().expect("联系人表锁中毒")
    }

    /// 改一份副本，写盘成功才换上：写不进去时内存和磁盘不分叉。
    fn commit(&self, change: impl FnOnce(&mut Stored)) -> Result<(), ContactFailure> {
        let mut stored = self.lock();
        let mut next = stored.clone();
        change(&mut next);
        if next == *stored {
            return Ok(());
        }
        store::write(&self.path, &next).map_err(|e| {
            tracing::error!(error = %e, "联系人记录写不进去");
            ContactFailure::new(ContactError::Upstream, e.to_string())
        })?;
        *stored = next;
        drop(stored);
        self.changes.send_modify(|n| *n += 1);
        Ok(())
    }

    /// 管这个 Bot 的是不是 `username`：私人 Bot 是归属人，共享 Bot 是主人。
    fn keeper_bot(&self, bot: &BotRef, username: &str) -> Result<LocalBot, ContactFailure> {
        let local = self.local_bot(bot)?;
        if local.keeper_username != username {
            return Err(ContactFailure::new(
                ContactError::NotKeeper,
                format!("{username} 不管 Bot {}/{}", bot.plugin_id, bot.bot),
            ));
        }
        Ok(local)
    }

    fn local_bot(&self, bot: &BotRef) -> Result<LocalBot, ContactFailure> {
        self.ports
            .directory
            .bot(bot)
            .map_err(|e| ContactFailure::new(ContactError::Upstream, e))?
            .ok_or_else(|| {
                ContactFailure::new(
                    ContactError::BotNotFound,
                    format!("没有 Bot {}/{}", bot.plugin_id, bot.bot),
                )
            })
    }

    /// 这个 Bot 的联系人，先查一遍还在等的人的邀请。只有管它的人能看。
    pub async fn list(
        &self,
        bot: &BotRef,
        username: &str,
    ) -> Result<Vec<BotContact>, ContactFailure> {
        let local = self.keeper_bot(bot, username)?;
        self.check_people_of(&local).await;
        Ok(self.view(bot))
    }

    /// 联系人页的列表：先是等归属人处理的请求，再是其他的；各自按时间排。
    pub fn view(&self, bot: &BotRef) -> Vec<BotContact> {
        let stored = self.lock();
        let mut rows: Vec<(&String, &Row)> = stored
            .rows(bot)
            .map(|rows| rows.iter().collect())
            .unwrap_or_default();
        rows.sort_by_key(|(user, row)| {
            (
                row.state != ContactState::Incoming,
                row.updated_at_ms,
                (*user).clone(),
            )
        });
        rows.into_iter()
            .map(|(user, row)| self.contact(user, row))
            .collect()
    }

    fn contact(&self, user_id: &str, row: &Row) -> BotContact {
        let remote = server_of(user_id)
            .filter(|server| *server != self.own_server)
            .map(|server| RemoteContact {
                server_name: server.to_owned(),
                peer_status: match self.ports.link.reach(server) {
                    PeerReach::Paired(status) => Some(status),
                    PeerReach::NotPaired => None,
                },
            });
        BotContact {
            user_id: user_id.to_owned(),
            display_name: row.display_name.clone(),
            keeper_display_name: row.keeper_display_name.clone(),
            kind: row.kind,
            state: row.state,
            remote,
        }
    }

    /// 等 `username` 处理的请求（“等你确认”列表），最早收到的在前。
    pub fn pending_for(&self, username: &str) -> Vec<PendingContactRequest> {
        let incoming: Vec<(BotRef, String, Row)> = self
            .lock()
            .all()
            .filter(|(_, _, row)| row.state == ContactState::Incoming)
            .map(|(bot, user, row)| (bot.clone(), user.to_owned(), row.clone()))
            .collect();
        let mut pending: Vec<PendingContactRequest> = incoming
            .into_iter()
            .filter(|(bot, _, _)| {
                matches!(self.ports.directory.bot(bot), Ok(Some(local)) if local.keeper_username == username)
            })
            .map(|(bot, user, row)| PendingContactRequest {
                contact: self.contact(&user, &row),
                bot,
                received_at_ms: row.updated_at_ms,
            })
            .collect();
        pending.sort_by_key(|request| request.received_at_ms);
        pending
    }

    /// 这个 Bot（按 Matrix 用户 ID）已成立的联系人：注入 `WONTON_MATRIX_ALLOWED_USERS` 时加在归属人后面。
    pub fn allowed_users(&self, bot_user_id: &str) -> Vec<String> {
        let Ok(Some(local)) = self.ports.directory.bot_by_user(bot_user_id) else {
            return Vec::new();
        };
        self.lock()
            .rows(&local.bot)
            .map(|rows| {
                rows.iter()
                    .filter(|(_, row)| row.state == ContactState::Active)
                    .map(|(user, _)| user.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `user_id` 在别的服务器上。认不出服务器名的也按远端算：证明不了是本机的人。
    pub(crate) fn is_remote(&self, user_id: &str) -> bool {
        server_of(user_id) != Some(self.own_server.as_str())
    }

    /// 这个 Bot 有没有已成立的远端联系人（人或 Bot 都算：kind 是对方核心自报的）。总线据此不给它插件工具
    /// （ADR-0050“远端联系人默认只聊天”）。认不出服务器名的用户 ID 也按远端算：证明不了是本机的人。
    pub fn has_active_remote(&self, bot: &BotRef) -> bool {
        self.lock().rows(bot).is_some_and(|rows| {
            rows.iter()
                .any(|(user, row)| row.state == ContactState::Active && self.is_remote(user))
        })
    }
}

pub(crate) fn now_ms() -> u64 {
    crate::peers::now_ms()
}

#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
mod tests_open_federation;
#[cfg(test)]
mod tests_remote;
