//! 没有自家服务器时的本机好友记录：外部 Matrix 账号的“好友”只是一份本机清单（ADR-0051 的 c 路）。
//! 有自家服务器时分组和墓碑都在服务器的 `friends.json` 里，这里不用。

use std::collections::BTreeMap;
use std::path::Path;

use crate::l10n::text;
use serde::{Deserialize, Serialize};
use wonton_protocol::FriendGroup;

const FILE: &str = "friends-local.json";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Local {
    #[serde(default)]
    pub groups: Vec<FriendGroup>,
    /// 每个人在哪几个组；删好友后留着，再加回来重新选。
    #[serde(default)]
    pub assigned: BTreeMap<String, Vec<String>>,
    /// 删过的人和删除时刻；私聊里的好友标记不算数，直到删除后重新收到邀请并同意。
    #[serde(default)]
    pub removed: BTreeMap<String, u64>,
    #[serde(default)]
    next: u64,
}

impl Local {
    pub fn open(dir: &Path) -> Result<Self, std::io::Error> {
        match std::fs::read(dir.join(FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(std::io::Error::other),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn groups_of(&self, user_id: &str) -> Vec<String> {
        self.assigned.get(user_id).cloned().unwrap_or_default()
    }

    pub fn save_group(&mut self, id: Option<&str>, name: &str) -> Result<FriendGroup, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(text("friends-group-name-required", &[]));
        }
        if let Some(id) = id {
            let group = self
                .groups
                .iter_mut()
                .find(|group| group.id == id)
                .ok_or_else(|| text("friends-group-not-found", &[("id", id.to_owned().into())]))?;
            group.name = name.to_owned();
            return Ok(group.clone());
        }
        self.next += 1;
        let group = FriendGroup {
            id: format!("group-{}", self.next),
            name: name.to_owned(),
        };
        self.groups.push(group.clone());
        Ok(group)
    }

    pub fn remove_group(&mut self, id: &str) {
        self.groups.retain(|group| group.id != id);
        for groups in self.assigned.values_mut() {
            groups.retain(|group| group != id);
        }
    }

    pub fn set_groups(&mut self, user_id: &str, groups: &[String]) {
        if groups.is_empty() {
            self.assigned.remove(user_id);
        } else {
            self.assigned.insert(user_id.to_owned(), groups.to_vec());
        }
    }

    pub fn save(&self, dir: &Path) -> Result<(), std::io::Error> {
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join(FILE), bytes)
    }
}
