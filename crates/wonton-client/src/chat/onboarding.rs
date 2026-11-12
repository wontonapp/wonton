//! 聊天列表为引导做的事：默认 Agent 的房间置顶，发过消息的房间记下来给引导判断提示用。

use std::sync::{Mutex, MutexGuard};

use tokio::sync::watch;
use wonton_protocol::RoomRow;

pub(crate) struct RoomOrder {
    base: Mutex<Vec<RoomRow>>,
}

impl RoomOrder {
    pub(crate) fn new() -> Self {
        Self {
            base: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn replace(&self, mut rows: Vec<RoomRow>, bot: Option<&str>) -> Vec<RoomRow> {
        *lock(&self.base) = rows.clone();
        prioritize(&mut rows, bot);
        rows
    }

    pub(crate) fn display(&self, bot: Option<&str>) -> Vec<RoomRow> {
        let mut rows = lock(&self.base).clone();
        prioritize(&mut rows, bot);
        rows
    }

    pub(crate) fn clear(&self) {
        lock(&self.base).clear();
    }
}

pub(crate) fn prioritize(rows: &mut Vec<RoomRow>, bot: Option<&str>) {
    let Some(bot) = bot else { return };
    if let Some(index) = rows
        .iter()
        .position(|row| row.direct_user_id.as_deref() == Some(bot))
    {
        let row = rows.remove(index);
        rows.insert(0, row);
    }
}

pub(crate) fn record_sent(sender: &watch::Sender<Vec<String>>, room_id: String) {
    sender.send_modify(|rooms| {
        if !rooms.iter().any(|sent| sent == &room_id) {
            rooms.push(room_id);
        }
    });
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::{RoomMembership, RoomRow};

    fn row(id: &str, direct_user_id: Option<&str>) -> RoomRow {
        RoomRow {
            layout: Default::default(),
            room_id: id.to_owned(),
            name: id.to_owned(),
            is_direct: direct_user_id.is_some(),
            direct_user_id: direct_user_id.map(str::to_owned),
            is_encrypted: false,
            is_read_only: false,
            membership: RoomMembership::Joined,
            preview: None,
            unread_count: 0,
        }
    }

    #[test]
    fn primary_bot_moves_to_front_without_reordering_the_rest() {
        let order = RoomOrder::new();
        let rows = order.replace(
            vec![
                row("one", None),
                row("bot", Some("@bot:home")),
                row("two", None),
            ],
            None,
        );
        assert_eq!(
            rows.iter()
                .map(|row| row.room_id.as_str())
                .collect::<Vec<_>>(),
            ["one", "bot", "two"]
        );
        let rows = order.display(Some("@bot:home"));
        assert_eq!(
            rows.iter()
                .map(|row| row.room_id.as_str())
                .collect::<Vec<_>>(),
            ["bot", "one", "two"]
        );
    }
}
