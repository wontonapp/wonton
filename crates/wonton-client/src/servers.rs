//! 配过的服务器（ADR-0042）：每台一个槽位目录，`servers.json` 记着槽位、服务器 ID、自定义名字、编号和当前是哪台。
//!
//! 这里只管列表，不碰作用域：每个操作返回切到哪个槽位、哪些槽位的目录可以删，由 [`ClientCore`](crate::ClientCore)
//! 停掉旧作用域、建新的，再删目录。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_protocol::{CoreError, ServerName, ServerRow, ServersViewModel};

use crate::home::write_private;
use crate::l10n::{CATALOG, language};

const FILE: &str = "servers.json";
const SLOTS_DIR: &str = "servers";

pub(crate) struct ServerList {
    root: PathBuf,
    saved: Mutex<Saved>,
    view: watch::Sender<ServersViewModel>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Saved {
    /// 按添加的顺序。当前槽位一定在里面，配对前它的 `server_id` 为空。
    servers: Vec<Slot>,
    current: String,
    /// 正在添加时，取消后回到的那台。
    adding_from: Option<String>,
    next_slot: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Slot {
    slot: String,
    /// 服务器的隧道 endpoint ID。
    server_id: Option<String>,
    name: Option<String>,
    number: u32,
}

/// 一次改动之后要做的事。
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Change {
    /// 当前槽位换了：切到这个槽位。
    pub(crate) switch_to: Option<String>,
    /// 关掉旧作用域之后可以删掉的槽位目录。
    pub(crate) discard: Vec<String>,
}

impl ServerList {
    /// 读 `servers.json`；没有时从槽位 `1` 开始。读不懂时如实报错，不当成没配过：那样会顶掉手机上的数据。
    pub(crate) fn open(root: &Path) -> Result<Self, CoreError> {
        let path = root.join(FILE);
        let saved = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| CoreError::Storage {
                reason: format!("服务器列表 {} 已损坏：{e}", path.display()),
            })?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let mut saved = Saved::default();
                let slot = saved.new_slot();
                saved.current = slot;
                saved
            }
            Err(e) => return Err(storage_error(&path, &e)),
        };
        let list = Self {
            root: root.to_owned(),
            view: watch::Sender::new(saved.view()),
            saved: Mutex::new(saved),
        };
        list.save(&list.lock())?;
        Ok(list)
    }

    pub(crate) fn view(&self) -> ServersViewModel {
        self.view.borrow().clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<ServersViewModel> {
        self.view.subscribe()
    }

    pub(crate) fn current(&self) -> String {
        self.lock().current.clone()
    }

    /// 配在哪个槽位上；没配过（或已移除）时为空。`server_id` 是服务器的 endpoint ID。
    pub(crate) fn slot_of(&self, server_id: &str) -> Option<String> {
        self.lock()
            .servers
            .iter()
            .find(|slot| slot.server_id.as_deref() == Some(server_id))
            .map(|slot| slot.slot.clone())
    }

    pub(crate) fn slot_dir(&self, slot: &str) -> PathBuf {
        self.root.join(SLOTS_DIR).join(slot)
    }

    /// 删掉不用的槽位目录。调用前要先停掉用着它的作用域。
    pub(crate) fn discard(&self, slots: &[String]) -> Result<(), CoreError> {
        for slot in slots {
            let dir = self.slot_dir(slot);
            match fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(storage_error(&dir, &e)),
            }
        }
        Ok(())
    }

    /// `AddServer`：换到一个新槽位，壳看到没配对，显示配对页。已经在添加时什么都不做。
    pub(crate) fn add(&self) -> Result<Change, CoreError> {
        self.update(|saved| {
            if saved.adding_from.is_some() {
                return Ok(Change::default());
            }
            if saved.current_slot().server_id.is_none() {
                return Err(CoreError::InvalidInput {
                    reason: "先配好这一台，再添加别的服务器".to_owned(),
                });
            }
            let slot = saved.new_slot();
            saved.adding_from = Some(saved.current.clone());
            saved.current = slot.clone();
            Ok(Change {
                switch_to: Some(slot),
                discard: Vec::new(),
            })
        })
    }

    /// `CancelAddServer`：删掉配对中的槽位，回到原来那台。
    pub(crate) fn cancel_add(&self) -> Result<Change, CoreError> {
        self.update(|saved| {
            let Some(back) = saved.adding_from.clone() else {
                return Ok(Change::default());
            };
            Ok(saved.leave_for(back))
        })
    }

    /// `SwitchServer`。正在添加时，配对中的槽位一起删掉。
    pub(crate) fn switch(&self, id: &str) -> Result<Change, CoreError> {
        self.update(|saved| {
            saved.paired_slot(id)?;
            if id == saved.current {
                return Ok(Change::default());
            }
            Ok(saved.leave_for(id.to_owned()))
        })
    }

    pub(crate) fn rename(&self, id: &str, name: &str) -> Result<Change, CoreError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::InvalidInput {
                reason: "名字不能为空".to_owned(),
            });
        }
        self.update(|saved| {
            saved.paired_slot(id)?.name = Some(name.to_owned());
            Ok(Change::default())
        })
    }

    /// `RemoveServer`：只删手机上的数据。移除当前这台时切到剩下的第一台，一台不剩就换一个空槽位回到配对页。
    pub(crate) fn remove(&self, id: &str) -> Result<Change, CoreError> {
        self.update(|saved| {
            saved.paired_slot(id)?;
            if id != saved.current {
                saved.servers.retain(|slot| slot.slot != id);
                if saved.adding_from.as_deref() == Some(id) {
                    saved.adding_from = saved.first_paired();
                }
                return Ok(Change {
                    switch_to: None,
                    discard: vec![id.to_owned()],
                });
            }
            saved.servers.retain(|slot| slot.slot != id);
            let next = saved.first_paired().unwrap_or_else(|| saved.new_slot());
            Ok(saved.leave_for(next))
        })
    }

    /// 这个槽位配上了服务器 `server_id`（隧道连上后的 `HomeViewModel::server_id`）。同一台服务器以前配在别的槽位上的，
    /// 那个旧槽位要删掉，名字和编号留给新槽位。
    pub(crate) fn paired(&self, slot: &str, server_id: &str) -> Result<Change, CoreError> {
        self.update(|saved| {
            let Some(index) = saved.servers.iter().position(|s| s.slot == slot) else {
                return Ok(Change::default());
            };
            if saved.servers[index].server_id.as_deref() == Some(server_id) {
                return Ok(Change::default());
            }
            let old = saved
                .servers
                .iter()
                .find(|s| s.slot != slot && s.server_id.as_deref() == Some(server_id))
                .cloned();
            saved.servers[index].server_id = Some(server_id.to_owned());
            if saved.current == slot {
                saved.adding_from = None;
            }
            let Some(old) = old else {
                if saved.servers[index].number == 0 {
                    saved.servers[index].number = saved
                        .servers
                        .iter()
                        .filter(|s| s.server_id.is_some())
                        .count() as u32;
                }
                return Ok(Change::default());
            };
            if saved.servers[index].name.is_none() {
                saved.servers[index].name = old.name;
            }
            saved.servers[index].number = old.number;
            saved.servers.retain(|s| s.slot != old.slot);
            Ok(Change {
                switch_to: None,
                discard: vec![old.slot],
            })
        })
    }

    /// 改动成功、存了盘才更新 ViewModel；存不了盘时内存里也不改。
    fn update(
        &self,
        change: impl FnOnce(&mut Saved) -> Result<Change, CoreError>,
    ) -> Result<Change, CoreError> {
        let mut saved = self.lock();
        let mut next = saved.clone();
        let outcome = change(&mut next)?;
        self.save(&next)?;
        *saved = next;
        let view = saved.view();
        self.view.send_if_modified(|current| {
            let modified = *current != view;
            *current = view;
            modified
        });
        Ok(outcome)
    }

    fn save(&self, saved: &Saved) -> Result<(), CoreError> {
        fs::create_dir_all(&self.root).map_err(|e| storage_error(&self.root, &e))?;
        let path = self.root.join(FILE);
        let bytes = serde_json::to_vec_pretty(saved).expect("服务器列表总能序列化");
        write_private(&path, &bytes).map_err(|e| storage_error(&path, &e))
    }

    fn lock(&self) -> MutexGuard<'_, Saved> {
        self.saved
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Saved {
    fn new_slot(&mut self) -> String {
        self.next_slot = self.next_slot.max(1);
        let slot = self.next_slot.to_string();
        self.next_slot += 1;
        self.servers.push(Slot {
            slot: slot.clone(),
            server_id: None,
            name: None,
            number: 0,
        });
        slot
    }

    fn first_paired(&self) -> Option<String> {
        self.servers
            .iter()
            .find(|slot| slot.server_id.is_some())
            .map(|slot| slot.slot.clone())
    }

    fn current_slot(&self) -> &Slot {
        self.servers
            .iter()
            .find(|slot| slot.slot == self.current)
            .expect("当前槽位一直在列表里")
    }

    fn paired_slot(&mut self, id: &str) -> Result<&mut Slot, CoreError> {
        self.servers
            .iter_mut()
            .find(|slot| slot.slot == id && slot.server_id.is_some())
            .ok_or_else(|| CoreError::InvalidInput {
                // 现实里只会是点了已经移除的服务器发来的旧通知；槽位号对用户没有意义，不写进文案。
                reason: l10n!(CATALOG, language(), "servers-removed"),
            })
    }

    /// 离开当前槽位去 `next`。离开的是还没配对的槽位（添加到一半），它的目录一起删掉。
    fn leave_for(&mut self, next: String) -> Change {
        let left = std::mem::replace(&mut self.current, next.clone());
        self.adding_from = None;
        let mut discard = Vec::new();
        if left != next {
            let unpaired = self
                .servers
                .iter()
                .any(|slot| slot.slot == left && slot.server_id.is_none());
            if unpaired || !self.servers.iter().any(|slot| slot.slot == left) {
                discard.push(left.clone());
            }
            if unpaired {
                self.servers.retain(|slot| slot.slot != left);
            }
        }
        Change {
            switch_to: Some(next),
            discard,
        }
    }

    fn view(&self) -> ServersViewModel {
        let servers = self
            .servers
            .iter()
            .filter(|slot| slot.server_id.is_some())
            .map(|slot| ServerRow {
                id: slot.slot.clone(),
                name: slot.name.clone().map_or(
                    ServerName::Default {
                        number: slot.number,
                    },
                    |name| ServerName::Named { name },
                ),
                current: slot.slot == self.current,
            })
            .collect();
        ServersViewModel {
            servers,
            adding: self.adding_from.is_some(),
        }
    }
}

/// 当前这台的名字；只配了一台时为空，设置首页不用写名字。
pub(crate) fn current_name(servers: &ServersViewModel) -> Option<ServerName> {
    if servers.servers.len() < 2 {
        return None;
    }
    servers
        .servers
        .iter()
        .find(|row| row.current)
        .map(|row| row.name.clone())
}

fn storage_error(path: &Path, error: &io::Error) -> CoreError {
    CoreError::Storage {
        reason: format!("{}：{error}", path.display()),
    }
}
