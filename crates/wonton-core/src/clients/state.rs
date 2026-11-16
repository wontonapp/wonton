//! 客户端入口的记录快照和不可恢复的连接撤销（ADR-0021）。
//!
//! `PluginManager::changed` 同步刷新快照：一次关闭立即撤销旧守卫，之后重开创建新守卫，
//! 不依赖可能合并多次变化的异步通知。请求准入同时取得标签、开关和守卫，不在热路径读盘。

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::plugin::PluginId;
use crate::plugin::store::Store;

/// 插件入口的开关（ADR-0021）：局域网入口、公网入口，默认全关。存插件记录。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientsEntries {
    #[serde(default)]
    pub lan: bool,
    #[serde(default)]
    pub public: bool,
}

impl ClientsEntries {
    pub(crate) fn of(self, kind: Kind) -> bool {
        match kind {
            Kind::Lan => self.lan,
            Kind::Public => self.public,
        }
    }
}

/// 入口的种类：公网主机名 `<标签>.<后缀>`，局域网主机名 `<标签>.lan.<后缀>`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Lan,
    Public,
}

impl Kind {
    /// 入口容器写在 `X-Wonton-Entry` 里的值（`entry_haproxy` 的前端名）。
    pub(crate) fn header_value(self) -> &'static str {
        match self {
            Kind::Lan => "lan",
            Kind::Public => "public",
        }
    }
}

/// 同一快照里的目标、开关与连接守卫；关闭后守卫永远保持撤销。
pub(crate) struct Resolved {
    pub id: PluginId,
    pub switches: ClientsEntries,
    pub revoked: watch::Receiver<bool>,
}

struct Binding {
    id: PluginId,
    switches: ClientsEntries,
    guards: [watch::Sender<bool>; 2],
}

type Labels = BTreeMap<String, Option<Binding>>;

/// 记录无法确定或标签冲突，不能冒充「没有这个标签」。详情只记服务器日志。
#[derive(Debug)]
pub(crate) struct Unavailable;

pub(crate) struct State {
    labels: Mutex<Result<Labels, Unavailable>>,
}

impl State {
    pub(crate) fn new(store: &Store) -> Self {
        let state = Self {
            labels: Mutex::new(Ok(BTreeMap::new())),
        };
        state.refresh(store);
        state
    }

    pub(crate) fn resolve(&self, label: &str, kind: Kind) -> Result<Option<Resolved>, Unavailable> {
        let labels = self.labels.lock().expect("入口快照锁中毒");
        let labels = labels.as_ref().map_err(|_| Unavailable)?;
        let Some(binding) = labels.get(label) else {
            return Ok(None);
        };
        let binding = binding.as_ref().ok_or(Unavailable)?;
        Ok(Some(Resolved {
            id: binding.id.clone(),
            switches: binding.switches,
            revoked: binding.guards[guard_index(kind)].subscribe(),
        }))
    }

    /// 写记录的一方必须在返回成功前调用；持锁完成刷新与撤销，关后重开不会合并掉关闭。
    pub(crate) fn refresh(&self, store: &Store) {
        let mut labels = self.labels.lock().expect("入口快照锁中毒");
        let next = read_labels(store);
        let mut previous = std::mem::replace(&mut *labels, Err(Unavailable));
        if let (Ok(next), Ok(previous)) = (&next, &mut previous) {
            for (label, binding) in previous.iter_mut() {
                let Some(binding) = binding else { continue };
                for kind in [Kind::Lan, Kind::Public] {
                    let stays_open = next
                        .get(label)
                        .and_then(Option::as_ref)
                        .is_some_and(|next| {
                            next.id == binding.id
                                && next.switches.of(kind)
                                && binding.switches.of(kind)
                        });
                    if !stays_open {
                        binding.guards[guard_index(kind)].send_replace(true);
                    }
                }
            }
        } else if let Ok(previous) = &previous {
            for binding in previous.values().flatten() {
                for guard in &binding.guards {
                    guard.send_replace(true);
                }
            }
        }
        *labels = next.map(|mut next| {
            if let Ok(previous) = &mut previous {
                for (label, binding) in &mut next {
                    if let (Some(binding), Some(Some(old))) = (binding, previous.remove(label)) {
                        for kind in [Kind::Lan, Kind::Public] {
                            if old.id == binding.id
                                && old.switches.of(kind)
                                && binding.switches.of(kind)
                            {
                                binding.guards[guard_index(kind)] =
                                    old.guards[guard_index(kind)].clone();
                            }
                        }
                    }
                }
            }
            next
        });
    }
}

fn guard_index(kind: Kind) -> usize {
    match kind {
        Kind::Lan => 0,
        Kind::Public => 1,
    }
}

fn read_labels(store: &Store) -> Result<Labels, Unavailable> {
    let files = store.load_all().map_err(|error| {
        tracing::error!(%error, "Cannot read client entry records");
        Unavailable
    })?;
    let mut labels = BTreeMap::new();
    for file in files {
        let record = file.record.map_err(|error| {
            tracing::error!(%error, "Cannot read client entry record");
            Unavailable
        })?;
        let Some(label) = record.entry_label else {
            continue;
        };
        let binding = Binding {
            id: file.id,
            switches: record.clients_entries,
            guards: [record.clients_entries.lan, record.clients_entries.public]
                .map(|open| watch::channel(!open).0),
        };
        match labels.entry(label) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Some(binding));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                tracing::error!(label = %entry.key(), "Conflicting client entry labels");
                entry.insert(None);
            }
        }
    }
    Ok(labels)
}
