//! 经核心的请求计数。守卫跟着实际执行或响应体走，取消、错误和正常结束都归还计数。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;
use wonton_protocol::{BotActivityFacts, BotActivityReport};

use crate::plugin::PluginId;

#[derive(Default)]
struct Counts {
    model_requests: u32,
    tool_calls: u32,
}

pub(crate) struct BotActivity {
    counts: Mutex<BTreeMap<Arc<str>, Counts>>,
    revision: watch::Sender<u64>,
}

#[derive(Clone, Copy)]
enum Kind {
    Model,
    Tool,
}

pub(crate) struct Guard {
    activity: Arc<BotActivity>,
    plugin: Arc<str>,
    kind: Kind,
}

impl Default for BotActivity {
    fn default() -> Self {
        Self {
            counts: Mutex::new(BTreeMap::new()),
            revision: watch::Sender::new(0),
        }
    }
}

impl Counts {
    fn counter(&mut self, kind: Kind) -> &mut u32 {
        match kind {
            Kind::Model => &mut self.model_requests,
            Kind::Tool => &mut self.tool_calls,
        }
    }
}

impl BotActivity {
    pub(crate) fn start_model(self: &Arc<Self>, plugin: &PluginId) -> Guard {
        self.start(plugin, Kind::Model)
    }

    pub(crate) fn start_tool(self: &Arc<Self>, plugin: &PluginId) -> Guard {
        self.start(plugin, Kind::Tool)
    }

    fn start(self: &Arc<Self>, plugin: &PluginId, kind: Kind) -> Guard {
        let mut counts = self.counts.lock().expect("活动表锁中毒");
        let plugin = counts.get_key_value(plugin.as_str()).map_or_else(
            || Arc::<str>::from(plugin.as_str()),
            |(plugin, _)| plugin.clone(),
        );
        *counts.entry(plugin.clone()).or_default().counter(kind) += 1;
        self.changed();
        Guard {
            activity: self.clone(),
            plugin,
            kind,
        }
    }

    pub(crate) fn changed(&self) {
        self.revision.send_modify(|rev| *rev += 1);
    }

    pub(crate) fn changes(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    pub(crate) fn rev(&self) -> u64 {
        *self.revision.borrow()
    }

    pub(crate) fn report(
        &self,
        mut bots: Vec<BotActivityFacts>,
        waiting: &BTreeMap<String, u32>,
    ) -> BotActivityReport {
        let counts = self.counts.lock().expect("活动表锁中毒");
        for bot in &mut bots {
            if let Some(counts) = counts.get(bot.bot.plugin_id.as_str()) {
                bot.model_requests = counts.model_requests;
                bot.tool_calls = counts.tool_calls;
            }
            bot.waiting = waiting.get(&bot.bot.plugin_id).copied().unwrap_or_default();
        }
        BotActivityReport {
            rev: self.rev(),
            bots,
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let mut counts = self.activity.counts.lock().expect("活动表锁中毒");
        let entry = counts.get_mut(&self.plugin).expect("守卫持有计数");
        *entry.counter(self.kind) -= 1;
        if entry.model_requests == 0 && entry.tool_calls == 0 {
            counts.remove(&self.plugin);
        }
        self.activity.changed();
    }
}
