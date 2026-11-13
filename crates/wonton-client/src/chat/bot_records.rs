use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use wonton_protocol::{BotActivityFacts, BotRef, BotView, ChatLayoutSettings};

use super::bot_activity::Activity;
use super::bots::{Bots, Local, Record};
use super::{layout, lock};

impl Bots {
    pub(super) fn reconcile(self: &Arc<Self>) {
        let mut local = lock(&self.local);
        let old = self.view.borrow().bots.clone();
        let (views, refresh) = reconcile(&mut local, &old, &self.layouts.current(), || {
            self.refresh_serial.fetch_add(1, Ordering::SeqCst) + 1
        });
        self.view.send_modify(|view| view.bots = views);
        drop(local);
        self.update_activity();
        for id in refresh {
            self.refresh(&id);
        }
        self.refresh_web();
        self.changed.notify_one();
        self.layouts.touch();
    }
}

// Only the core's explicit name and Matrix ID establish an identity. InstalledPlugin's
// bot_user_id describes the default editor target, not the plugin's complete Bot list.
fn identity(facts: &BotActivityFacts) -> Option<BotRef> {
    let name = facts.bot.as_ref().filter(|name| !name.is_empty())?;
    if facts.plugin_id.is_empty() || facts.user_id.is_empty() {
        return None;
    }
    Some(BotRef {
        plugin_id: facts.plugin_id.clone(),
        bot: name.clone(),
    })
}

fn reconcile(
    local: &mut Local,
    old: &[BotView],
    settings: &ChatLayoutSettings,
    mut serial: impl FnMut() -> u64,
) -> (Vec<BotView>, Vec<BotRef>) {
    let mut records = std::mem::take(&mut local.records);
    let mut views = Vec::new();
    let mut refresh = Vec::new();
    let facts = local
        .report
        .as_ref()
        .map(|report| report.bots.as_slice())
        .unwrap_or_default();
    let mut counts = HashMap::new();
    for fact in facts {
        if let Some(name) = fact.bot.as_deref() {
            *counts
                .entry((fact.plugin_id.as_str(), name))
                .or_insert(0usize) += 1;
        }
    }
    for fact in facts {
        let Some(id) = identity(fact) else { continue };
        // Conflicting facts for one BotRef cannot select an arbitrary Matrix account.
        if counts.get(&(id.plugin_id.as_str(), id.bot.as_str())) != Some(&1) {
            continue;
        }
        let plugin = local.installed.as_ref().and_then(|installed| {
            installed
                .plugins
                .iter()
                .find(|plugin| plugin.id == id.plugin_id)
        });
        let title = plugin.map_or(id.plugin_id.as_str(), |plugin| plugin.title.as_str());
        let previous = records
            .remove(&id)
            .filter(|record| record.user_id == fact.user_id);
        let mut view = previous
            .as_ref()
            .and_then(|_| {
                old.iter()
                    .find(|view| view.bot == id && view.user_id == fact.user_id)
            })
            .cloned()
            .unwrap_or_else(|| BotView {
                bot: id.clone(),
                user_id: fact.user_id.clone(),
                web: None,
                name: title.to_owned(),
                avatar: None,
                state_avatars: Vec::new(),
                activity: None,
                saving: false,
                problem: None,
                chat: Default::default(),
            });
        view.chat = layout::bot_settings(
            settings,
            plugin.and_then(|plugin| plugin.chat.as_ref()),
            &fact.user_id,
        );
        let mut record = previous.unwrap_or_else(|| Record {
            user_id: fact.user_id.clone(),
            title: title.to_owned(),
            facts: None,
            profile_rev: None,
            typing: Default::default(),
            activity: Activity::default(),
            refresh: 0,
            identity: serial(),
        });
        record.title = title.to_owned();
        if record.profile_rev != Some(fact.profile_rev) {
            record.profile_rev = Some(fact.profile_rev);
            refresh.push(id.clone());
        }
        record.facts = Some(fact.clone());
        local.records.insert(id, record);
        views.push(view);
    }
    (views, refresh)
}

#[cfg(test)]
#[path = "bot_records_tests.rs"]
mod tests;
