//! 没有自家服务器时的本机好友记录：外部 Matrix 账号的“好友”只是一份本机清单（ADR-0051 的 c 路）。
//! 有自家服务器时好友和墓碑都在服务器的 `friends.json` 里，这里不用。

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

const FILE: &str = "friends-local.json";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Local {
    /// 删过的人和删除时刻；私聊里的好友标记不算数，直到删除后重新收到邀请并同意。
    #[serde(default)]
    pub removed: BTreeMap<String, u64>,
}

impl Local {
    pub fn open(dir: &Path) -> Result<Self, std::io::Error> {
        match std::fs::read(dir.join(FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(std::io::Error::other),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<(), std::io::Error> {
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join(FILE), bytes)
    }
}
