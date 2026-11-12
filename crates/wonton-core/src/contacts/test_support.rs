//! 联系人测试的替身：配对名单、Bot 名录、推送。只在测试里编（`mod.rs` 里 `#[cfg(test)]`）。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use tokio::sync::watch;
use wonton_protocol::{BotRef, ContactRequestPush};

use super::PeerDirectory;
use wonton_protocol::PeerStatus;

use super::{BotDirectory, ContactPush};

/// 配对名单的替身。`pair` 会抬 watch，生产行为一致（配对变化唤醒房间补建）。
pub struct FakePeers {
    names: watch::Sender<BTreeSet<String>>,
}

impl Default for FakePeers {
    fn default() -> Self {
        Self {
            names: watch::Sender::new(BTreeSet::new()),
        }
    }
}

impl FakePeers {
    pub fn pair(&self, server: &str) {
        self.names.send_modify(|names| {
            names.insert(server.to_owned());
        });
    }

    /// 取消配对：抬 watch（生产行为一致，收件循环靠它清 outbox）。
    pub fn unpair(&self, server: &str) {
        self.names.send_modify(|names| {
            names.remove(server);
        });
    }
}

impl PeerDirectory for FakePeers {
    fn paired(&self, server_name: &str) -> bool {
        self.names.borrow().contains(server_name)
    }

    fn status(&self, server_name: &str) -> Option<PeerStatus> {
        self.paired(server_name).then_some(PeerStatus::Connected)
    }

    fn paired_names(&self) -> watch::Receiver<BTreeSet<String>> {
        self.names.subscribe()
    }
}

/// Bot 名录的替身：登记同机的 Bot，重建动作记下来给断言。
#[derive(Default)]
pub struct FakeBots {
    users: Mutex<BTreeMap<String, BotRef>>,
    display_names: Mutex<BTreeMap<String, String>>,
    keeper_names: Mutex<BTreeMap<String, String>>,
    keeper_users: Mutex<BTreeMap<String, String>>,
    localparts: Mutex<BTreeMap<String, String>>,
    rebuilds: Mutex<Vec<String>>,
}

impl FakeBots {
    /// 登记一个同机 Bot：Matrix 用户 ID、显示名、归属人的显示名和用户名。
    pub fn add(&self, user: &str, bot: &BotRef, display: &str, keeper_display: &str, keeper: &str) {
        self.users
            .lock()
            .unwrap()
            .insert(user.to_owned(), bot.clone());
        self.display_names
            .lock()
            .unwrap()
            .insert(user.to_owned(), display.to_owned());
        self.keeper_names
            .lock()
            .unwrap()
            .insert(user.to_owned(), keeper_display.to_owned());
        self.keeper_users
            .lock()
            .unwrap()
            .insert(user.to_owned(), keeper.to_owned());
        self.localparts
            .lock()
            .unwrap()
            .insert(user.to_owned(), bot.bot.clone());
    }

    pub fn rebuilds(&self) -> Vec<String> {
        self.rebuilds.lock().unwrap().clone()
    }
}

impl BotDirectory for FakeBots {
    fn bot_of_user(&self, user_id: &str) -> Option<BotRef> {
        self.users.lock().unwrap().get(user_id).cloned()
    }

    fn user_of_bot(&self, bot: &BotRef) -> Option<String> {
        self.users
            .lock()
            .unwrap()
            .iter()
            .find(|(_, reference)| **reference == *bot)
            .map(|(user, _)| user.clone())
    }

    fn bot_localpart(&self, bot: &BotRef) -> Option<String> {
        let user = self.user_of_bot(bot)?;
        self.localparts.lock().unwrap().get(&user).cloned()
    }

    fn display_name(&self, bot: &BotRef) -> Option<String> {
        let user = self.user_of_bot(bot)?;
        self.display_names.lock().unwrap().get(&user).cloned()
    }

    fn keeper_display_name(&self, bot: &BotRef) -> Option<String> {
        let user = self.user_of_bot(bot)?;
        self.keeper_names.lock().unwrap().get(&user).cloned()
    }

    fn keeper_username(&self, bot: &BotRef) -> Option<String> {
        let user = self.user_of_bot(bot)?;
        self.keeper_users.lock().unwrap().get(&user).cloned()
    }

    fn rebuild_bot(&self, bot: &BotRef) {
        let user = self.user_of_bot(bot).unwrap_or_default();
        self.rebuilds.lock().unwrap().push(user);
    }
}

/// 推送的替身：记下每条发出去的请求。
#[derive(Default)]
pub struct FakePush {
    sent: Mutex<Vec<(Option<String>, ContactRequestPush)>>,
}

impl FakePush {
    pub fn requests(&self) -> Vec<(Option<String>, ContactRequestPush)> {
        self.sent.lock().unwrap().clone()
    }
}

impl ContactPush for FakePush {
    fn contact_request(&self, keeper: Option<&str>, push: &ContactRequestPush) {
        self.sent
            .lock()
            .unwrap()
            .push((keeper.map(str::to_owned), push.clone()));
    }
}
