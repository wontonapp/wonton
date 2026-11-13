//! 客户端入口的开关状态（ADR-0021）：开关存插件记录（照 `lan_webui` 的存法），标签就是记录里的
//! `entry_label`。这里按入口转发的节奏读：标签 → 插件记录，缓存到插件表的下一次变化为止。
//! 关开关的接口在 C7；它改完记录后照例唤醒插件表（`PluginManager::changed`），监视任务据此把
//! 这个入口的 watch 置位，正在转发的连接（含已升级的 WebSocket）立即断。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::plugin::PluginId;
use crate::plugin::PluginManager;

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

/// 按标签解析出的一个插件入口。
pub(crate) struct Resolved {
    pub id: PluginId,
    pub switches: ClientsEntries,
}

struct LabelMap {
    generation: u64,
    by_label: BTreeMap<String, (PluginId, ClientsEntries)>,
}

/// 客户端入口的状态：标签解析缓存和每个入口的连接守卫。挂在网关上，生命周期同网关。
pub(crate) struct State {
    manager: Arc<PluginManager>,
    changes: watch::Receiver<u64>,
    labels: Mutex<LabelMap>,
    watches: Mutex<BTreeMap<(String, Kind), watch::Sender<bool>>>,
    /// 监视任务只起一次：第一次有连接要守卫时才需要它。
    monitor: AtomicBool,
}

impl State {
    pub(crate) fn new(manager: Arc<PluginManager>) -> Self {
        let changes = manager.changes();
        Self {
            manager,
            changes,
            labels: Mutex::new(LabelMap {
                generation: 0,
                by_label: BTreeMap::new(),
            }),
            watches: Mutex::new(BTreeMap::new()),
            monitor: AtomicBool::new(false),
        }
    }

    /// 标签 → 插件和它的开关。缓存到插件表的下一次变化为止：装、卸、升级、开关都会唤醒插件表。
    pub(crate) fn resolve(&self, label: &str) -> Option<Resolved> {
        let generation = *self.changes.borrow();
        {
            let labels = self.labels.lock().expect("标签缓存锁中毒");
            if labels.generation == generation
                && let Some((id, switches)) = labels.by_label.get(label)
            {
                return Some(Resolved {
                    id: id.clone(),
                    switches: *switches,
                });
            }
        }
        let mut by_label = BTreeMap::new();
        if let Ok(files) = self.manager.store().load_all() {
            for file in files {
                let Ok(record) = file.record else {
                    continue;
                };
                let Some(label) = record.entry_label else {
                    continue;
                };
                if by_label
                    .insert(label.clone(), (file.id, record.clients_entries))
                    .is_some()
                {
                    // 安装时会避开重名（entry_haproxy::labels），撞上了说明记录被外部改过：后一个不动开关，
                    // 提醒排查，不猜谁是真身。
                    tracing::warn!(%label, "两个插件记录用了同一个入口标签，开关以后者为准前存疑");
                }
            }
        }
        let resolved = by_label.get(label).map(|(id, switches)| Resolved {
            id: id.clone(),
            switches: *switches,
        });
        *self.labels.lock().expect("标签缓存锁中毒") = LabelMap {
            generation,
            by_label,
        };
        resolved
    }

    /// 这个入口的连接守卫：开关关闭（或插件没了）时置位。每个插件、每个入口一个 watch，照
    /// `gateway/sharing/guard.rs` 的守卫；转发层收到置位就断，已升级的连接立即断。
    pub(crate) fn watch(self: &Arc<Self>, label: &str, kind: Kind) -> watch::Receiver<bool> {
        self.ensure_monitor();
        let key = (label.to_owned(), kind);
        let mut watches = self.watches.lock().expect("入口 watch 锁中毒");
        if let Some(sender) = watches.get(&key) {
            return sender.subscribe();
        }
        let (sender, _) = watch::channel(false);
        watches.insert(key, sender.clone());
        sender.subscribe()
    }

    /// 监视插件表：哪个入口的开关关了（或插件没了、记录读不出），把它的 watch 置位并从表里删掉，
    /// 之后重新打开会拿到新的 watch。
    fn ensure_monitor(self: &Arc<Self>) {
        if self.monitor.swap(true, Ordering::SeqCst) {
            return;
        }
        let state = self.clone();
        tokio::spawn(async move {
            let mut changes = state.manager.changes();
            loop {
                if changes.changed().await.is_err() {
                    return;
                }
                let keys: Vec<(String, Kind)> = state
                    .watches
                    .lock()
                    .expect("入口 watch 锁中毒")
                    .keys()
                    .cloned()
                    .collect();
                for (label, kind) in keys {
                    let open = state
                        .resolve(&label)
                        .is_some_and(|resolved| resolved.switches.of(kind));
                    if !open
                        && let Some(sender) = state
                            .watches
                            .lock()
                            .expect("入口 watch 锁中毒")
                            .remove(&(label.clone(), kind))
                    {
                        sender.send_replace(true);
                    }
                }
            }
        });
    }
}
