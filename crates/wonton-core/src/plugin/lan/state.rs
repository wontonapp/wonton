//! 核心里局域网设置的状态。主人的选择存在插件记录目录下（[`crate::plugin::store::Store::lan_choice`]），这里是
//! 怎么检测、`WONTON_LAN_ADDRESSES`，以及检测之间要记住的事。

use std::collections::BTreeSet;
use std::net::IpAddr;
use std::sync::Mutex;

use tokio::sync::watch;

use super::detect::Detector;
use crate::plugin::manifest::PluginId;

pub struct Lan {
    pub(super) detector: Detector,
    /// `WONTON_LAN_ADDRESSES`：设了就只用这些地址，设置里不能改。
    pub(super) fixed: Option<Vec<IpAddr>>,
    /// 定期检测、设置页和改选择各自触发检测，一次一次来，不同时重建同一个插件。
    pub(super) pass: tokio::sync::Mutex<()>,
    /// 最近一次选出的局域网地址。检测失败时留着上次的：查不出来不等于地址没了。
    pub(super) addresses: watch::Sender<BTreeSet<IpAddr>>,
    memory: Mutex<Memory>,
}

#[derive(Default)]
struct Memory {
    /// 上次检测的结论，变了才写日志。
    last: Option<String>,
    /// 正在按新地址重建的插件：这时卸载如实报“正在进行”。
    rebuilding: BTreeSet<PluginId>,
    /// 因为局域网标为异常的插件（选不出地址、重建失败，或者核心启动时核对没过）：每次检测都重新处理，好了就恢复。
    degraded: BTreeSet<PluginId>,
}

impl Lan {
    pub fn new(detector: Detector, fixed: Option<Vec<IpAddr>>) -> Self {
        Self {
            detector,
            fixed,
            pass: tokio::sync::Mutex::new(()),
            addresses: watch::Sender::new(BTreeSet::new()),
            memory: Mutex::new(Memory::default()),
        }
    }

    /// 核心在容器里时起检测容器，直接跑在宿主上时直接读。
    pub fn for_core(core_container: Option<String>) -> Self {
        let detector = match core_container {
            Some(core) => Detector::Container { core },
            None => Detector::Here,
        };
        Self::new(detector, None)
    }

    /// 只用这些地址（`WONTON_LAN_ADDRESSES`）。
    #[must_use]
    pub fn with_fixed(mut self, addresses: Vec<IpAddr>) -> Self {
        self.fixed = Some(addresses);
        self
    }

    fn memory(&self) -> std::sync::MutexGuard<'_, Memory> {
        self.memory.lock().expect("局域网状态锁中毒")
    }

    /// 记下这次检测的结论，返回和上次比变了没有。
    pub(super) fn changed(&self, summary: String) -> bool {
        let mut memory = self.memory();
        let changed = memory.last.as_ref() != Some(&summary);
        memory.last = Some(summary);
        changed
    }

    pub(in crate::plugin) async fn lock_pass(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.pass.lock().await
    }
    pub fn is_rebuilding(&self, id: &PluginId) -> bool {
        self.memory().rebuilding.contains(id)
    }

    /// 记为正在重建，已经在重建时返回 `false`。
    pub(in crate::plugin) fn begin(&self, id: &PluginId) -> bool {
        self.memory().rebuilding.insert(id.clone())
    }

    pub(in crate::plugin) fn end(&self, id: &PluginId) {
        self.memory().rebuilding.remove(id);
    }

    pub fn mark_degraded(&self, id: &PluginId) {
        self.memory().degraded.insert(id.clone());
    }

    pub fn clear_degraded(&self, id: &PluginId) {
        self.memory().degraded.remove(id);
    }

    pub(in crate::plugin) fn is_degraded(&self, id: &PluginId) -> bool {
        self.memory().degraded.contains(id)
    }
}
