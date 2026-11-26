//! 客户端入口的记录快照和不可恢复的连接撤销（ADR-0021）。
//!
//! `PluginManager::changed` 同步刷新快照：一次关闭立即撤销旧守卫，之后重开创建新守卫，
//! 不依赖可能合并多次变化的异步通知。请求准入同时取得标签、开关、规范对外地址和守卫，不在热路径读盘。

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::clients::external_url;
use crate::config::PublicUrls;
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

/// 同一快照里的目标、开关、规范对外地址与连接守卫；关闭后守卫永远保持撤销。
pub(crate) struct Resolved {
    pub id: PluginId,
    pub switches: ClientsEntries,
    pub revoked: watch::Receiver<bool>,
    /// 公开入口改写 `Host` 用的规范地址（ADR-0089 第 5 节），快照里算好。
    pub external_url: String,
}

struct Binding {
    id: PluginId,
    switches: ClientsEntries,
    guards: [watch::Sender<bool>; 2],
    external_url: String,
}

#[derive(Default)]
struct Snapshot {
    labels: BTreeMap<String, Option<Binding>>,
    /// 每个插件算好的规范对外地址：隧道请求没有标签，按插件 id 查。
    external_urls: BTreeMap<PluginId, String>,
}

/// 记录无法确定或标签冲突，不能冒充「没有这个标签」。详情只记服务器日志。
#[derive(Debug)]
pub(crate) struct Unavailable;

pub(crate) struct State {
    snapshot: Mutex<Result<Snapshot, Unavailable>>,
}

impl State {
    pub(crate) fn new(store: &Store, public: &PublicUrls) -> Self {
        let state = Self {
            snapshot: Mutex::new(Ok(Snapshot::default())),
        };
        state.refresh(store, None, public);
        state
    }

    pub(crate) fn resolve(&self, label: &str, kind: Kind) -> Result<Option<Resolved>, Unavailable> {
        let snapshot = self.snapshot.lock().expect("入口快照锁中毒");
        let snapshot = snapshot.as_ref().map_err(|_| Unavailable)?;
        let Some(binding) = snapshot.labels.get(label) else {
            return Ok(None);
        };
        let binding = binding.as_ref().ok_or(Unavailable)?;
        Ok(Some(Resolved {
            id: binding.id.clone(),
            switches: binding.switches,
            revoked: binding.guards[guard_index(kind)].subscribe(),
            external_url: binding.external_url.clone(),
        }))
    }

    /// 这个插件当前的规范对外地址（隧道请求按插件 id 查）；记录读不到时没有。
    pub(crate) fn external_url(&self, id: &PluginId) -> Option<String> {
        let snapshot = self.snapshot.lock().ok()?;
        snapshot.as_ref().ok()?.external_urls.get(id).cloned()
    }

    /// 写记录或改后缀的一方必须在返回成功前调用；持锁完成刷新与撤销，关后重开不会合并掉关闭。
    /// `suffix` 只影响规范对外地址，和开关无关（ADR-0089 第 5 节）。
    pub(crate) fn refresh(&self, store: &Store, suffix: Option<&str>, public: &PublicUrls) {
        let mut snapshot = self.snapshot.lock().expect("入口快照锁中毒");
        let next = read_snapshot(store, suffix, public);
        let mut previous = std::mem::replace(&mut *snapshot, Err(Unavailable));
        if let (Ok(next), Ok(previous)) = (&next, &mut previous) {
            for (label, binding) in previous.labels.iter_mut() {
                let Some(binding) = binding else { continue };
                for kind in [Kind::Lan, Kind::Public] {
                    let stays_open =
                        next.labels
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
            for binding in previous.labels.values().flatten() {
                for guard in &binding.guards {
                    guard.send_replace(true);
                }
            }
        }
        *snapshot = next.map(|mut next| {
            if let Ok(previous) = &mut previous {
                for (label, binding) in &mut next.labels {
                    if let (Some(binding), Some(Some(old))) =
                        (binding, previous.labels.remove(label))
                    {
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

fn read_snapshot(
    store: &Store,
    suffix: Option<&str>,
    public: &PublicUrls,
) -> Result<Snapshot, Unavailable> {
    let files = store.load_all().map_err(|error| {
        tracing::error!(%error, "Cannot read client entry records");
        Unavailable
    })?;
    let mut snapshot = Snapshot::default();
    for file in files {
        let record = file.record.map_err(|error| {
            tracing::error!(%error, "Cannot read client entry record");
            Unavailable
        })?;
        let app_url = external_url::app_origin(public, &file.id);
        let value = external_url::value(
            record.clients_entries,
            record.entry_label.as_deref(),
            suffix,
            Some(&app_url),
        )
        .unwrap_or(app_url);
        snapshot
            .external_urls
            .insert(file.id.clone(), value.clone());
        let Some(label) = record.entry_label else {
            continue;
        };
        let binding = Binding {
            id: file.id,
            switches: record.clients_entries,
            guards: [record.clients_entries.lan, record.clients_entries.public]
                .map(|open| watch::channel(!open).0),
            external_url: value,
        };
        match snapshot.labels.entry(label) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Some(binding));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                tracing::error!(label = %entry.key(), "Conflicting client entry labels");
                entry.insert(None);
            }
        }
    }
    Ok(snapshot)
}
