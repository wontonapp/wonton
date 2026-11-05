//! 主人对确认的放宽（ADR-0024）：一律允许、始终允许、标成只读。判定在总线（`crate::mcp`），这里只管存和查。
//!
//! 文件读不出来时当作没有任何放宽，写请求也拒绝，不覆盖它：坏文件里可能还有主人的设置。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use wonton_protocol::BotSwitch;

use super::languages::Languages;
use super::manager::PluginManager;
use super::manifest::{LocalizedText, Permission, PluginId};
use super::store::write_json;

/// 主人的设置，存盘的格式。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Settings {
    /// 一律允许：这些 Bot 调所有插件都不问。
    #[serde(default)]
    allow_all: BTreeSet<String>,
    /// 始终允许：插件 → 调它不问的 Bot。
    #[serde(default)]
    always: BTreeMap<String, BTreeSet<String>>,
    /// 标成只读：插件 → 工具。
    #[serde(default)]
    read_only: BTreeMap<String, BTreeSet<String>>,
}

enum State {
    Ready(Settings),
    Unreadable(String),
}

/// 这次调用不用问主人的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clearance {
    OwnerReadOnly,
    AllowAll,
    Always,
}

/// 设置改不了。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("工具策略不可用：{0}")]
pub struct PolicyUnavailable(pub String);

pub struct ToolPolicy {
    path: PathBuf,
    state: Mutex<State>,
}

impl ToolPolicy {
    pub fn load(path: PathBuf) -> Self {
        let state = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(settings) => State::Ready(settings),
                Err(e) => unreadable(&path, &e),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::Ready(Settings::default()),
            Err(e) => unreadable(&path, &e),
        };
        Self {
            path,
            state: Mutex::new(state),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("工具策略锁中毒")
    }

    /// `caller` 调 `plugin` 的 `tool` 不用问主人的原因；没有就要问。
    pub fn clearance(&self, caller: &str, plugin: &str, tool: &str) -> Option<Clearance> {
        let state = self.lock();
        let State::Ready(settings) = &*state else {
            return None;
        };
        if listed(&settings.read_only, plugin, tool) {
            Some(Clearance::OwnerReadOnly)
        } else if settings.allow_all.contains(caller) {
            Some(Clearance::AllowAll)
        } else if listed(&settings.always, plugin, caller) {
            Some(Clearance::Always)
        } else {
            None
        }
    }

    /// 文件读不出来的原因。
    pub fn problem(&self) -> Option<String> {
        match &*self.lock() {
            State::Ready(_) => None,
            State::Unreadable(reason) => Some(reason.clone()),
        }
    }

    pub fn is_allow_all(&self, caller: &str) -> bool {
        self.read(|settings| settings.allow_all.contains(caller))
    }

    pub fn is_always(&self, plugin: &str, caller: &str) -> bool {
        self.read(|settings| listed(&settings.always, plugin, caller))
    }

    pub fn is_read_only(&self, plugin: &str, tool: &str) -> bool {
        self.read(|settings| listed(&settings.read_only, plugin, tool))
    }

    fn read(&self, query: impl FnOnce(&Settings) -> bool) -> bool {
        match &*self.lock() {
            State::Ready(settings) => query(settings),
            State::Unreadable(_) => false,
        }
    }

    pub fn set_allow_all(&self, caller: &str, on: bool) -> Result<(), PolicyUnavailable> {
        self.change(|settings| toggle(&mut settings.allow_all, caller, on))
    }

    pub fn set_always(
        &self,
        plugin: &str,
        caller: &str,
        on: bool,
    ) -> Result<(), PolicyUnavailable> {
        self.change(|settings| toggle_in(&mut settings.always, plugin, caller, on))
    }

    pub fn set_read_only(
        &self,
        plugin: &str,
        tool: &str,
        on: bool,
    ) -> Result<(), PolicyUnavailable> {
        self.change(|settings| toggle_in(&mut settings.read_only, plugin, tool, on))
    }

    /// 插件被卸载（保留数据也算）：它的只读标记、别的 Bot 对它的始终允许，和它作为 Bot 得到的放宽一起清掉。
    pub fn forget_plugin(&self, id: &str) {
        let forgotten = self.change(|settings| {
            settings.allow_all.remove(id);
            settings.always.remove(id);
            settings.read_only.remove(id);
            for callers in settings.always.values_mut() {
                callers.remove(id);
            }
            settings.always.retain(|_, callers| !callers.is_empty());
        });
        if let Err(e) = forgotten {
            tracing::error!(plugin = %id, error = %e, "清掉插件的工具策略失败");
        }
    }

    /// 先写盘，写成了才生效：放宽不能只在内存里，收回也不能重启后又回来。
    fn change(&self, edit: impl FnOnce(&mut Settings)) -> Result<(), PolicyUnavailable> {
        let mut state = self.lock();
        let settings = match &*state {
            State::Ready(settings) => settings,
            State::Unreadable(reason) => return Err(PolicyUnavailable(reason.clone())),
        };
        let mut changed = settings.clone();
        edit(&mut changed);
        if &changed == settings {
            return Ok(());
        }
        write_json(&self.path, &changed).map_err(|e| PolicyUnavailable(e.to_string()))?;
        *state = State::Ready(changed);
        Ok(())
    }
}

fn unreadable(path: &std::path::Path, error: &dyn std::fmt::Display) -> State {
    let reason = format!("读不出 {}：{error}", path.display());
    tracing::error!("{reason}");
    State::Unreadable(reason)
}

fn listed(map: &BTreeMap<String, BTreeSet<String>>, key: &str, value: &str) -> bool {
    map.get(key).is_some_and(|values| values.contains(value))
}

fn toggle(set: &mut BTreeSet<String>, value: &str, on: bool) {
    if on {
        set.insert(value.to_owned());
    } else {
        set.remove(value);
    }
}

fn toggle_in(map: &mut BTreeMap<String, BTreeSet<String>>, key: &str, value: &str, on: bool) {
    let values = map.entry(key.to_owned()).or_default();
    toggle(values, value, on);
    if values.is_empty() {
        map.remove(key);
    }
}

impl PluginManager {
    pub fn tool_policy(&self) -> &ToolPolicy {
        self.store.tool_policy()
    }

    /// Bot：申请了 `tools`、能调工具的插件。一个插件一个 Bot，调用方就是插件 ID（ADR-0024）。
    pub fn tool_callers(&self) -> Vec<(PluginId, LocalizedText)> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .filter(|(_, entry)| {
                entry
                    .listed
                    .as_ref()
                    .is_some_and(|listed| listed.manifest.permissions.contains(&Permission::Tools))
            })
            .map(|(id, entry)| (id.clone(), entry.title.clone()))
            .collect()
    }

    /// 已经对这个插件始终允许的 Bot，放在升级确认页上。
    pub(super) fn always_allowed(&self, id: &PluginId, langs: &Languages) -> Vec<BotSwitch> {
        let policy = self.tool_policy();
        self.tool_callers()
            .into_iter()
            .filter(|(caller, _)| policy.is_always(id.as_str(), caller.as_str()))
            .map(|(caller, title)| BotSwitch {
                caller: caller.to_string(),
                name: langs.pick_owned(&title),
                enabled: true,
            })
            .collect()
    }

    /// 插件声明了 `mcp`，不管在不在运行。
    pub fn declares_mcp(&self, id: &PluginId) -> bool {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .get(id)
            .and_then(|entry| entry.listed.as_ref())
            .is_some_and(|listed| listed.manifest.mcp.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> (ToolPolicy, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (ToolPolicy::load(dir.path().join("policy.json")), dir)
    }

    #[test]
    fn settings_survive_a_restart() {
        let (policy, dir) = policy();
        policy.set_always("notes", "hermes", true).unwrap();
        policy.set_read_only("notes", "unread", true).unwrap();
        let reopened = ToolPolicy::load(dir.path().join("policy.json"));
        assert_eq!(
            reopened.clearance("hermes", "notes", "delete"),
            Some(Clearance::Always)
        );
        assert_eq!(
            reopened.clearance("other", "notes", "unread"),
            Some(Clearance::OwnerReadOnly)
        );
        assert_eq!(reopened.clearance("other", "notes", "delete"), None);
    }

    #[test]
    fn an_unreadable_file_means_no_relaxation_and_stays_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.json");
        std::fs::write(&path, "不是 JSON").unwrap();
        let policy = ToolPolicy::load(path.clone());
        assert_eq!(policy.clearance("hermes", "notes", "delete"), None);
        assert!(policy.problem().is_some());
        assert!(policy.set_allow_all("hermes", true).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "不是 JSON");
    }

    #[test]
    fn a_change_that_cannot_be_written_does_not_take_effect() {
        let dir = tempfile::tempdir().unwrap();
        let policy = ToolPolicy::load(dir.path().join("missing/policy.json"));
        assert!(policy.set_allow_all("hermes", true).is_err());
        assert_eq!(policy.clearance("hermes", "notes", "delete"), None);
    }

    #[test]
    fn uninstalling_a_plugin_clears_it_on_both_sides() {
        let (policy, _dir) = policy();
        policy.set_always("notes", "hermes", true).unwrap();
        policy.set_always("mail", "hermes", true).unwrap();
        policy.set_always("mail", "other", true).unwrap();
        policy.set_allow_all("hermes", true).unwrap();
        policy.set_read_only("notes", "delete", true).unwrap();
        policy.forget_plugin("hermes");
        assert_eq!(policy.clearance("hermes", "mail", "send"), None);
        assert_eq!(
            policy.clearance("other", "mail", "send"),
            Some(Clearance::Always)
        );
        policy.forget_plugin("notes");
        assert_eq!(policy.clearance("other", "notes", "delete"), None);
    }
}
