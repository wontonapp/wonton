//! `core/contacts.json` 的形状和它上面的状态机。行按 `<插件 ID>/<Bot 名>` 分组，一组里按用户 ID 一行。

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use wonton_protocol::{BotRef, ContactEvent, ContactKind, ContactOutcome, ContactState};

fn bot_key(bot: &BotRef) -> String {
    format!("{}/{}", bot.plugin_id, bot.bot)
}

/// 待发送的出站事件：先落意图再发（ADR-0050）。发送成功就从这里删掉；失败留着，收件循环按退避重试。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxEvent {
    pub server: String,
    pub event: ContactEvent,
    pub txn: String,
    pub enqueued_at_ms: u64,
}

/// 联系人记录的落盘形状。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    pub bots: BTreeMap<String, Vec<StoredContact>>,
    /// 核心联系人房间：对方服务器名 → 房间 ID，重启后不用靠全量同步找回来。
    pub rooms: BTreeMap<String, String>,
    /// 没送出去的出站事件，按入队顺序发。
    #[serde(default)]
    pub outbox: Vec<OutboxEvent>,
}

/// 一行联系人。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredContact {
    pub user_id: String,
    pub kind: ContactKind,
    pub state: ContactState,
    /// 远端的联系人带对方服务器名；同机的没有。
    pub remote: Option<String>,
    /// 对方的显示名：同机的发起时从本机名录取，远端的自报值随请求来。
    pub display_name: Option<String>,
    /// 对方是私人 Bot 时归属人的显示名（远端的自报）。
    pub keeper_display_name: Option<String>,
    /// 这一行挂着的请求：跨机的按它对结果（旧请求迟到的结果不算数），同机的只有一方有。
    pub request_id: Option<String>,
    /// 对人的邀请：Bot 发的私聊房间，接受前就在。
    pub room: Option<String>,
    pub updated_at_ms: u64,
}

/// 读 `core/contacts.json`；还没有这个文件就是空表。
pub fn read(path: &Path) -> std::io::Result<Stored> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Stored::default()),
        Err(e) => Err(e),
    }
}

/// 原子落盘：写临时文件再改名，中途断电不留半份。
pub fn write(path: &Path, stored: &Stored) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(stored)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// 存储键 `<插件 ID>/<Bot 名>` 的还原。
pub(crate) fn parse_key(key: &str) -> BotRef {
    let (plugin_id, bot) = key.split_once('/').unwrap_or((key, ""));
    BotRef {
        plugin_id: plugin_id.to_owned(),
        bot: bot.to_owned(),
    }
}

/// 远端发来的一条联系人请求要落表的字段（[`Stored::on_remote_request`]）。
pub struct RemoteRequest<'a> {
    pub bot: &'a BotRef,
    pub from_user: &'a str,
    pub from_server: &'a str,
    pub request_id: &'a str,
    pub display_name: Option<String>,
    pub keeper_display_name: Option<String>,
}

/// 远端事件核对通过后的处理结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Take {
    /// 记下了（新请求或再发的请求）。
    Recorded,
    /// 两边互相发了请求，直接成立；调用方还要把同意的结果送回对方。
    Established,
    /// 整条忽略。
    Ignore,
}

/// 远端发来的一条结果要落的字段（[`Stored::on_remote_result`]）。Bot 由事件里的 `to` 认定。
pub struct RemoteResult<'a> {
    pub bot: &'a BotRef,
    pub from_user: &'a str,
    pub from_server: &'a str,
    pub request_id: &'a str,
    pub outcome: ContactOutcome,
    pub keeper_display_name: Option<String>,
}

impl Stored {
    /// 一个 Bot 的行，没有时是空表。
    pub fn rows(&self, bot: &BotRef) -> Vec<StoredContact> {
        self.bots.get(&bot_key(bot)).cloned().unwrap_or_default()
    }

    pub fn row(&self, bot: &BotRef, user_id: &str) -> Option<StoredContact> {
        self.rows(bot).into_iter().find(|c| c.user_id == user_id)
    }

    /// 放一行：已有的按用户 ID 换掉。返回存下的行。
    pub fn upsert(&mut self, bot: &BotRef, contact: StoredContact) -> StoredContact {
        let rows = self.bots.entry(bot_key(bot)).or_default();
        match rows.iter_mut().find(|c| c.user_id == contact.user_id) {
            Some(existing) => *existing = contact.clone(),
            None => rows.push(contact.clone()),
        }
        contact
    }

    pub fn remove(&mut self, bot: &BotRef, user_id: &str) -> Option<StoredContact> {
        let rows = self.bots.get_mut(&bot_key(bot))?;
        let index = rows.iter().position(|c| c.user_id == user_id)?;
        Some(rows.remove(index))
    }

    /// 远端发来请求（ADR-0050）。返回要不要往外送结果：两边互相发了请求时直接成立，我方把结果送回去；
    /// 已是联系人的重复请求、核对不过的都返回 [`Take::Ignore`]，事件整条忽略。
    pub fn on_remote_request(&mut self, request: &RemoteRequest, at_ms: u64) -> Take {
        let existing = self.row(request.bot, request.from_user);
        let incoming = StoredContact {
            user_id: request.from_user.to_owned(),
            kind: ContactKind::Bot,
            state: ContactState::Incoming,
            remote: Some(request.from_server.to_owned()),
            display_name: request.display_name.clone(),
            keeper_display_name: request.keeper_display_name.clone(),
            request_id: Some(request.request_id.to_owned()),
            room: None,
            updated_at_ms: at_ms,
        };
        match existing {
            Some(row) if row.state == ContactState::Active => Take::Ignore,
            Some(row) if row.state == ContactState::Pending => {
                self.upsert(
                    request.bot,
                    StoredContact {
                        state: ContactState::Active,
                        updated_at_ms: at_ms,
                        ..incoming
                    },
                );
                Take::Established
            }
            _ => {
                self.upsert(request.bot, incoming);
                Take::Recorded
            }
        }
    }

    /// 远端发来结果：事件里的 `to` 认定本机 Bot，行按“发起方用户 + 服务器 + 请求 ID”认（ADR-0050），
    /// 三者对不上整条忽略——别的 Bot 不会有同名远端联系人被误改。
    pub fn on_remote_result(&mut self, result: &RemoteResult, at_ms: u64) -> Option<ContactState> {
        let rows = self.bots.get_mut(&bot_key(result.bot))?;
        let row = rows.iter_mut().find(|c| {
            c.user_id == result.from_user
                && c.remote.as_deref() == Some(result.from_server)
                && c.request_id.as_deref() == Some(result.request_id)
        })?;
        if row.state != ContactState::Pending {
            return None;
        }
        row.state = match result.outcome {
            ContactOutcome::Approved => ContactState::Active,
            ContactOutcome::Rejected => ContactState::Declined,
            ContactOutcome::NotFound => ContactState::NotFound,
            // 比这台核心新的核心发的结果，整条忽略。
            ContactOutcome::Other => return None,
        };
        if let Some(keeper) = &result.keeper_display_name {
            row.keeper_display_name = Some(keeper.clone());
        }
        row.updated_at_ms = at_ms;
        Some(row.state)
    }

    /// 远端删了联系人或取消了请求：事件里的 `to` 认定本机 Bot，行按“发起方用户 + 服务器”认。
    /// 两个本机 Bot 都有同一个远端联系人时，只删 `to` 指的那一个。
    pub fn on_remote_remove(
        &mut self,
        bot: &BotRef,
        from_user: &str,
        from_server: &str,
    ) -> Option<StoredContact> {
        let rows = self.bots.get_mut(&bot_key(bot))?;
        let index = rows
            .iter()
            .position(|c| c.user_id == from_user && c.remote.as_deref() == Some(from_server))?;
        Some(rows.remove(index))
    }

    /// 归属人同意一条进来的请求：只有 Incoming 能落成 Active，别的状态没有可同意的。
    pub fn approve(
        &mut self,
        bot: &BotRef,
        user_id: &str,
        at_ms: u64,
    ) -> Result<StoredContact, ()> {
        let row = self.row(bot, user_id).ok_or(())?;
        if row.state != ContactState::Incoming {
            return Err(());
        }
        Ok(self.upsert(
            bot,
            StoredContact {
                state: ContactState::Active,
                updated_at_ms: at_ms,
                ..row
            },
        ))
    }

    /// 远端的人进了邀请的房间：只有对人的 Pending 行能这样落成 Active。
    pub fn person_joined(
        &mut self,
        bot: &BotRef,
        user_id: &str,
        at_ms: u64,
    ) -> Result<StoredContact, ()> {
        let row = self.row(bot, user_id).ok_or(())?;
        if row.kind != ContactKind::Person || row.state != ContactState::Pending {
            return Err(());
        }
        Ok(self.upsert(
            bot,
            StoredContact {
                state: ContactState::Active,
                updated_at_ms: at_ms,
                ..row
            },
        ))
    }

    /// 记下联系人房间（对方服务器名 → 房间 ID）。
    pub fn set_peer_room(&mut self, server_name: &str, room: &str) {
        self.rooms.insert(server_name.to_owned(), room.to_owned());
    }

    pub fn peer_room(&self, server_name: &str) -> Option<String> {
        self.rooms.get(server_name).cloned()
    }

    /// 出站事件入队：和行的改动在同一个落盘里，先落意图再发。
    pub fn enqueue(&mut self, server: &str, event: ContactEvent, txn: String, at_ms: u64) {
        self.outbox.push(OutboxEvent {
            server: server.to_owned(),
            event,
            txn,
            enqueued_at_ms: at_ms,
        });
    }

    /// 按序取一台服务器上能发的事件（发送在调用方做，发成一条删一条）。
    pub fn outbox_for(&self, server: &str) -> Vec<OutboxEvent> {
        self.outbox
            .iter()
            .filter(|entry| entry.server == server)
            .cloned()
            .collect()
    }

    /// 发成功一条：从队列里删掉。
    pub fn dequeue(&mut self, txn: &str) {
        self.outbox.retain(|entry| entry.txn != txn);
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
