//! “我来操作”（ADR-0024）：主人在插件网页上操作期间，Bot 的敏感调用先停住，不发卡片也不执行，操作结束后回到判定。
//! 只看内存：核心重启后没人在操作，App 下一次续约就重新开始。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::model::CallToolResult;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::watch;
use tokio::time::Instant;

use super::audit::{Decision, Record, TakeoverRecord};
use super::{Bus, Caller, error, with_failure};
use crate::plugin::PluginId;

/// 客户端每 10 秒续一次，这么久没续就算结束：隧道断了、App 被杀时没有人来说一声。
const LEASE: Duration = Duration::from_secs(30);
/// 每个调用方最多停住这么多调用。停住的调用不发卡片，防刷屏的上限（ADR-0016）管不到它们。
const MAX_HELD_PER_CALLER: usize = 16;
/// 给 Agent 看的类别：明确没有执行，和结果不确定分开。
const OWNER_OPERATING: &str = "owner_operating";

/// “我来操作”怎么结束的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TakeoverEnd {
    /// 主人交还：点了按钮、离开网页或者 App 进了后台。
    Handed,
    /// 没人续约：隧道断了或者 App 被杀。
    LeaseLapsed,
    DeviceRevoked,
    PluginStopped,
}

#[derive(Default)]
struct Table {
    /// 正在操作的插件：谁在操作，各自的租约到什么时候。几台设备同时操作时，最后一台结束才算结束。
    operating: HashMap<PluginId, HashMap<String, Instant>>,
    /// 每个调用方停住了几个调用。
    held: HashMap<PluginId, usize>,
}

pub(super) struct Takeovers {
    lease: Mutex<Duration>,
    table: Mutex<Table>,
    /// 有插件的操作结束时响一下，等着的调用自己再看一眼。
    ended: watch::Sender<()>,
}

/// 占着一个停住的名额，调用不再停住时（包括 Agent 断开、这次请求被丢弃）还回去。
struct Slot<'a> {
    takeovers: &'a Takeovers,
    caller: PluginId,
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        let mut table = self.takeovers.lock();
        if let Some(count) = table.held.get_mut(&self.caller) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                table.held.remove(&self.caller);
            }
        }
    }
}

impl Takeovers {
    pub(super) fn new() -> Self {
        Self {
            lease: Mutex::new(LEASE),
            table: Mutex::default(),
            ended: watch::Sender::new(()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Table> {
        self.table.lock().expect("操作表锁中毒")
    }

    /// 开始或续约。这个插件原来没人在操作时返回 `true`。
    fn renew(&self, plugin: &PluginId, by: &str) -> bool {
        let until = Instant::now() + *self.lease.lock().expect("租约锁中毒");
        let mut table = self.lock();
        let holders = table.operating.entry(plugin.clone()).or_default();
        let started = holders.is_empty();
        holders.insert(by.to_owned(), until);
        started
    }

    /// 从这个插件上去掉满足条件的操作者。最后一个走了就结束，返回他。
    fn remove_holders(
        &self,
        plugin: &PluginId,
        leaves: impl Fn(&str, &Instant) -> bool,
    ) -> Option<String> {
        let mut table = self.lock();
        let holders = table.operating.get_mut(plugin)?;
        let mut last = None;
        holders.retain(|by, until| {
            let leaving = leaves(by, until);
            if leaving {
                last = Some(by.clone());
            }
            !leaving
        });
        if !holders.is_empty() {
            return None;
        }
        table.operating.remove(plugin);
        drop(table);
        self.ended.send_replace(());
        last
    }

    fn next_lapse(&self, plugin: &PluginId) -> Option<Instant> {
        self.lock().operating.get(plugin)?.values().min().copied()
    }

    fn is_operating(&self, plugin: &PluginId) -> bool {
        self.lock().operating.contains_key(plugin)
    }

    fn operating_plugins(&self) -> Vec<PluginId> {
        self.lock().operating.keys().cloned().collect()
    }

    fn take_slot(&self, caller: &PluginId) -> Option<Slot<'_>> {
        let mut table = self.lock();
        let count = table.held.entry(caller.clone()).or_default();
        if *count >= MAX_HELD_PER_CALLER {
            return None;
        }
        *count += 1;
        Some(Slot {
            takeovers: self,
            caller: caller.clone(),
        })
    }
}

impl Bus {
    /// 主人开始操作或者续约。`by` 是哪台设备或主人令牌。
    pub fn renew_takeover(self: &Arc<Self>, plugin: &PluginId, by: &str) {
        if self.takeovers.renew(plugin, by) {
            self.audit_takeover("started", plugin, by, None);
            self.watch_lease(plugin.clone());
        }
    }

    /// 主人交还。这台设备本来没在操作时什么也不做，交还可以重复发。
    pub fn end_takeover(&self, plugin: &PluginId, by: &str) {
        self.end_holders(plugin, TakeoverEnd::Handed, |holder, _| holder == by);
    }

    /// 设备被吊销：它正在操作的插件不再算它。
    pub fn end_device_takeovers(&self, by: &str) {
        for plugin in self.takeovers.operating_plugins() {
            self.end_holders(&plugin, TakeoverEnd::DeviceRevoked, |holder, _| {
                holder == by
            });
        }
    }

    /// 能不能操作：插件在运行，而且声明了工具。别的插件 Bot 本来就调不到。
    pub fn serves_tools(&self, plugin: &PluginId) -> bool {
        self.plugins.mcp_servers().iter().any(|s| &s.id == plugin)
    }

    /// 插件表变了：停了或者卸载了的插件不再算有人在操作。
    pub(super) fn end_stopped_takeovers(&self) {
        let running = self.plugins.mcp_servers();
        for plugin in self.takeovers.operating_plugins() {
            if !running.iter().any(|s| s.id == plugin) {
                self.end_holders(&plugin, TakeoverEnd::PluginStopped, |_, _| true);
            }
        }
    }

    fn end_holders(
        &self,
        plugin: &PluginId,
        reason: TakeoverEnd,
        leaves: impl Fn(&str, &Instant) -> bool,
    ) {
        if let Some(by) = self.takeovers.remove_holders(plugin, leaves) {
            self.audit_takeover("ended", plugin, &by, Some(reason));
        }
    }

    /// 到点就去掉没续约的操作者。交还之后马上又开始时会有两个任务，结束只算一次，多的那个下一轮就退出。
    fn watch_lease(self: &Arc<Self>, plugin: PluginId) {
        let bus = self.clone();
        tokio::spawn(async move {
            while let Some(at) = bus.takeovers.next_lapse(&plugin) {
                tokio::time::sleep_until(at).await;
                let now = Instant::now();
                bus.end_holders(&plugin, TakeoverEnd::LeaseLapsed, |_, until| *until <= now);
            }
        });
    }

    fn audit_takeover(
        &self,
        takeover: &'static str,
        plugin: &PluginId,
        by: &str,
        reason: Option<TakeoverEnd>,
    ) {
        self.audit.record(&TakeoverRecord {
            takeover,
            plugin: plugin.as_str(),
            by,
            reason,
        });
    }

    /// 主人正在操作这个插件就先停住，等操作结束。返回停了多久，没停过是 `None`；没等到时返回给 Agent 的结果。
    pub(super) async fn hold(
        &self,
        caller: &Caller,
        name: &str,
        args: &Value,
        plugin: &PluginId,
        mut cancelled: Pin<&mut (impl Future<Output = ()> + Send)>,
    ) -> Result<Option<u64>, CallToolResult> {
        // 先订阅再看：看完到开始等之间结束的，也叫得醒。
        let mut ended = self.takeovers.ended.subscribe();
        if !self.takeovers.is_operating(plugin) {
            return Ok(None);
        }
        let Some(_slot) = self.takeovers.take_slot(&caller.id) else {
            return Err(self.not_run(
                caller,
                name,
                args,
                "主人正在操作这个插件，等着的调用太多了，这次调用没有执行。",
            ));
        };
        let started = Instant::now();
        let deadline = tokio::time::sleep(self.confirmations.ttl());
        tokio::pin!(deadline);
        while self.takeovers.is_operating(plugin) {
            tokio::select! {
                // 发送方在这个结构体里，收不到错误。
                _ = ended.changed() => {}
                () = &mut deadline => {
                    return Err(self.not_run(
                        caller,
                        name,
                        args,
                        "主人操作这个插件的时间太长，这次调用没有执行，过一会再试。",
                    ));
                }
                () = cancelled.as_mut() => return Err(self.withdrawn(caller, name, args)),
            }
        }
        let held = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(Some(held))
    }

    fn not_run(&self, caller: &Caller, name: &str, args: &Value, message: &str) -> CallToolResult {
        self.audit.record(&Record {
            caller: caller.id.as_str(),
            tool: name,
            arguments: args,
            decision: Decision::Unconfirmed,
            basis: None,
            outcome: Some(OWNER_OPERATING),
            files: &[],
        });
        with_failure(message, OWNER_OPERATING)
    }

    /// Agent 在停住期间断开：没有卡片，主人没答应过，调用作废。
    fn withdrawn(&self, caller: &Caller, name: &str, args: &Value) -> CallToolResult {
        let message = "调用方断开了，这次调用没有执行。";
        self.audit.record(&Record {
            caller: caller.id.as_str(),
            tool: name,
            arguments: args,
            decision: Decision::Withdrawn,
            basis: None,
            outcome: Some(message),
            files: &[],
        });
        error(message)
    }

    #[cfg(test)]
    pub(super) fn set_takeover_lease(&self, lease: Duration) {
        *self.takeovers.lease.lock().expect("租约锁中毒") = lease;
    }
}
