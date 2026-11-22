//! 房间里的斜杠命令表（ADR-0054，[ADR-0109](../../../docs/adr/0109-chat-like-telegram.md) §8）：读各 Bot 写的状态事件，
//! 合成给壳画的菜单。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::runtime::Handle;
use tokio::sync::watch;
use wonton_matrix::{MatrixSession, OwnedRoomId};
use wonton_protocol::{BOT_COMMANDS_EVENT, BotCommands, RoomCommand};

use super::lock;

/// 房间有动静时最多多久对一次表：命令表不在 Sync 的必需状态里，收到状态事件本身不会叫醒时间线。
const REFRESH_INTERVAL: Duration = Duration::from_secs(10);

/// 一个房间的命令表。表来自房间状态，只认成员自己写的。
pub(super) struct Tables {
    room_id: OwnedRoomId,
    inner: Mutex<Inner>,
    changed: watch::Sender<u64>,
}

#[derive(Default)]
struct Inner {
    generation: u64,
    last: Option<Instant>,
    commands: Vec<RoomCommand>,
    omitted: u32,
    /// 上一批送出去的：一样的表不重复发。
    sent: (Vec<RoomCommand>, u32),
}

impl Tables {
    pub(super) fn new(room_id: OwnedRoomId) -> Arc<Self> {
        Arc::new(Self {
            room_id,
            inner: Mutex::default(),
            changed: watch::channel(0).0,
        })
    }

    pub(super) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// 现在的表和因校验丢掉的条数。
    pub(super) fn values(&self) -> (Vec<RoomCommand>, u32) {
        let inner = lock(&self.inner);
        (inner.commands.clone(), inner.omitted)
    }

    /// 表和上一批送出去的不一样就记下这一份并返回真。
    pub(super) fn take_changed(&self) -> bool {
        let mut inner = lock(&self.inner);
        let next = (inner.commands.clone(), inner.omitted);
        if inner.sent == next {
            return false;
        }
        inner.sent = next;
        true
    }

    /// 向服务器重取一次这个房间的状态，隔 [`REFRESH_INTERVAL`] 最多一次。取不到时保留上一次的表，只记日志：
    /// 读不到表只该少个菜单，不该让时间线出错。
    pub(super) fn refresh(self: &Arc<Self>, runtime: &Handle, session: &Arc<MatrixSession>) {
        let generation = {
            let mut inner = lock(&self.inner);
            let now = Instant::now();
            if inner
                .last
                .is_some_and(|last| now.duration_since(last) < REFRESH_INTERVAL)
            {
                return;
            }
            inner.last = Some(now);
            inner.generation += 1;
            inner.generation
        };
        let tables = self.clone();
        let session = session.clone();
        runtime.spawn(async move {
            match session.room_state(&tables.room_id).await {
                // 先发的可能后回来，只认最后一次取的结果。
                Ok(_) if lock(&tables.inner).generation != generation => {}
                Ok(state) => tables.set(&state),
                Err(error) => {
                    tracing::info!(room_id = %tables.room_id, "couldn't read the bot command tables: {error}");
                }
            }
        });
    }

    fn set(&self, state: &[Value]) {
        let (commands, omitted) = commands(state);
        let mut inner = lock(&self.inner);
        if inner.commands == commands && inner.omitted == omitted {
            return;
        }
        inner.commands = commands;
        inner.omitted = omitted;
        self.changed
            .send_modify(|count| *count = count.wrapping_add(1));
    }
}

/// 从房间状态里合成命令表：`state_key` 和发送者相同的、发送者还在房间里的才算；每张表过 [`BotCommands::usable`]。
/// 结果按 Bot 进房间的先后排（同一天进来的按账号名），`omitted` 是各表自己截掉的加校验丢掉的条数。
pub(super) fn commands(state: &[Value]) -> (Vec<RoomCommand>, u32) {
    let mut joined: HashMap<&str, (&str, u64)> = HashMap::new();
    let mut bots: Vec<(&str, &str, u64, Vec<RoomCommand>)> = Vec::new();
    let mut omitted = 0u32;
    for event in state {
        if event["type"] == "m.room.member" && event["content"]["membership"] == "join" {
            let Some(user) = event["state_key"].as_str() else {
                continue;
            };
            let name = match event["content"]["displayname"].as_str() {
                Some(name) if !name.trim().is_empty() => name,
                _ => localpart(user),
            };
            let joined_at = event["origin_server_ts"].as_u64().unwrap_or(0);
            joined.entry(user).or_insert((name, joined_at));
        }
    }
    for event in state
        .iter()
        .filter(|event| event["type"] == BOT_COMMANDS_EVENT)
    {
        let (Some(sender), Some(_)) = (event["sender"].as_str(), event["state_key"].as_str())
        else {
            continue;
        };
        if event["state_key"].as_str() != Some(sender) {
            continue;
        }
        let Some((name, joined_at)) = joined.get(sender) else {
            continue;
        };
        let Ok(table) = serde_json::from_value::<BotCommands>(event["content"].clone()) else {
            tracing::warn!(bot = sender, "skipping an unreadable bot command table");
            continue;
        };
        let usable = table.usable();
        let dropped = u32::try_from(table.commands.len() - usable.len()).unwrap_or(u32::MAX);
        omitted = omitted
            .saturating_add(table.omitted)
            .saturating_add(dropped);
        let commands = usable
            .into_iter()
            .map(|command| RoomCommand {
                bot_user_id: sender.to_owned(),
                bot_name: (*name).to_owned(),
                text: format!("/{}", command.name),
                name: command.name,
                description: command.description,
                hint: command.hint,
            })
            .collect();
        bots.push((sender, name, *joined_at, commands));
    }
    bots.sort_by(|left, right| left.2.cmp(&right.2).then_with(|| left.0.cmp(right.0)));
    let commands = bots
        .into_iter()
        .flat_map(|(_, _, _, commands)| commands)
        .collect();
    (commands, omitted)
}

fn localpart(user: &str) -> &str {
    user.trim_start_matches('@')
        .split(':')
        .next()
        .unwrap_or(user)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn member(user: &str, name: &str, at: u64) -> Value {
        json!({"type": "m.room.member", "state_key": user, "sender": user, "origin_server_ts": at,
               "content": {"membership": "join", "displayname": name}})
    }

    fn table(sender: &str, key: &str, names: &[&str]) -> Value {
        json!({"type": BOT_COMMANDS_EVENT, "sender": sender, "state_key": key,
               "content": {"commands": names.iter().map(|name| json!({"name": name, "description": "d"})).collect::<Vec<_>>()}})
    }

    /// 行为：两个 Bot 的表按进房间的先后合成，重名的都留，各自带自己的名字。
    #[test]
    fn tables_of_both_bots_keep_their_order() {
        let state = vec![
            member("@b:home.example", "B", 20),
            member("@a:home.example", "A", 10),
            table("@b:home.example", "@b:home.example", &["go"]),
            table("@a:home.example", "@a:home.example", &["go", "help"]),
        ];
        let (commands, omitted) = commands(&state);
        assert_eq!(omitted, 0);
        let seen: Vec<_> = commands
            .iter()
            .map(|command| (command.bot_name.as_str(), command.text.as_str()))
            .collect();
        assert_eq!(seen, [("A", "/go"), ("A", "/help"), ("B", "/go")]);
    }

    /// 行为：别人替 Bot 写的、没进房间的人写的、解析不了的表都跳过，不合规的条目算丢掉。
    #[test]
    fn foreign_and_invalid_tables_are_dropped() {
        let state = vec![
            member("@a:home.example", "A", 10),
            // 别人替 A 写。
            table("@owner:home.example", "@a:home.example", &["forged"]),
            // 不在房间里的人写的。
            table("@gone:home.example", "@gone:home.example", &["gone"]),
            // 解析不了。
            json!({"type": BOT_COMMANDS_EVENT, "sender": "@a:home.example", "state_key": "@a:home.example",
                   "content": {"commands": "nope"}}),
            // 合规的和不合规的各一条。
            table("@a:home.example", "@a:home.example", &["ok", "/slash"]),
        ];
        let (commands, omitted) = commands(&state);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].text, "/ok");
        assert_eq!(omitted, 1);
    }
}
