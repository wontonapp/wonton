//! 主人的工具策略：对确认的放宽（ADR-0024：一律允许、始终允许、标成只读），和对单个 Bot 关掉的插件（ADR-0066 第 5 节）。
//! 判定在总线（`crate::mcp`），这里只管存和查。
//!
//! 文件读不出来时当作没有任何放宽、也不知道哪些插件对哪个 Bot 关了，写请求也拒绝，不覆盖它：坏文件里可能还有主人的设置。

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
    /// 单独关掉：插件 → 对它关了的 Bot（[`bot_key`]）。没记的 Bot 按插件的范围。
    #[serde(default)]
    bot_off: BTreeMap<String, BTreeSet<String>>,
}

/// 一个插件可以有几个 Bot（ADR-0045），单独关掉按 Bot 记。插件 ID 和 Bot 名里都没有 `/`。
fn bot_key(plugin: &str, bot: &str) -> String {
    format!("{plugin}/{bot}")
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

    /// `plugin` 的 Bot `bot` 被主人单独关掉了 `target` 的工具。文件读不出来时不知道，报错，由调用方拒绝。
    pub fn is_bot_off(
        &self,
        plugin: &str,
        bot: &str,
        target: &str,
    ) -> Result<bool, PolicyUnavailable> {
        match &*self.lock() {
            State::Ready(settings) => Ok(listed(&settings.bot_off, target, &bot_key(plugin, bot))),
            State::Unreadable(reason) => Err(PolicyUnavailable(reason.clone())),
        }
    }

    pub fn set_bot_off(
        &self,
        plugin: &str,
        bot: &str,
        target: &str,
        off: bool,
    ) -> Result<(), PolicyUnavailable> {
        let key = bot_key(plugin, bot);
        self.change(|settings| toggle_in(&mut settings.bot_off, target, &key, off))
    }

    /// Bot 被删：它的单独设置一起删（ADR-0066 第 5 节）。
    pub fn forget_bot(&self, plugin: &str, bot: &str) {
        let key = bot_key(plugin, bot);
        let forgotten = self.change(|settings| {
            for bots in settings.bot_off.values_mut() {
                bots.remove(&key);
            }
            settings.bot_off.retain(|_, bots| !bots.is_empty());
        });
        if let Err(e) = forgotten {
            tracing::error!(plugin, bot, error = %e, "清掉 Bot 的单独设置失败");
        }
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

    /// 插件被卸载（保留数据也算）：它的只读标记、别的 Bot 对它的始终允许和单独关掉，和它作为 Bot 得到的放宽、
    /// 它的 Bot 的单独关掉一起清掉。
    pub fn forget_plugin(&self, id: &str) {
        let prefix = bot_key(id, "");
        let forgotten = self.change(|settings| {
            settings.allow_all.remove(id);
            settings.always.remove(id);
            settings.read_only.remove(id);
            settings.bot_off.remove(id);
            for callers in settings.always.values_mut() {
                callers.remove(id);
            }
            settings.always.retain(|_, callers| !callers.is_empty());
            for bots in settings.bot_off.values_mut() {
                bots.retain(|key| !key.starts_with(&prefix));
            }
            settings.bot_off.retain(|_, bots| !bots.is_empty());
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

    /// 主人对 Bot 单独开关一个插件。改成了就通知总线上的 Agent 重新取工具列表。
    pub fn set_bot_plugin_access(
        &self,
        plugin: &PluginId,
        bot: &str,
        target: &PluginId,
        enabled: bool,
    ) -> Result<(), PolicyUnavailable> {
        self.tool_policy()
            .set_bot_off(plugin.as_str(), bot, target.as_str(), !enabled)?;
        self.changed();
        Ok(())
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

    #[test]
    fn a_bot_switched_off_stays_off_across_a_restart_and_only_for_that_bot() {
        let (policy, dir) = policy();
        assert_eq!(
            policy.is_bot_off("app.agent", "alice", "app.notes"),
            Ok(false)
        );
        policy
            .set_bot_off("app.agent", "alice", "app.notes", true)
            .unwrap();
        let reopened = ToolPolicy::load(dir.path().join("policy.json"));
        assert_eq!(
            reopened.is_bot_off("app.agent", "alice", "app.notes"),
            Ok(true)
        );
        assert_eq!(
            reopened.is_bot_off("app.agent", "bob", "app.notes"),
            Ok(false)
        );
        assert_eq!(
            reopened.is_bot_off("app.agent", "alice", "app.mail"),
            Ok(false)
        );
        reopened
            .set_bot_off("app.agent", "alice", "app.notes", false)
            .unwrap();
        assert_eq!(
            reopened.is_bot_off("app.agent", "alice", "app.notes"),
            Ok(false)
        );
    }

    #[test]
    fn removing_a_bot_or_either_plugin_clears_its_switches() {
        let (policy, _dir) = policy();
        policy
            .set_bot_off("app.agent", "alice", "app.notes", true)
            .unwrap();
        policy
            .set_bot_off("app.agent", "bob", "app.notes", true)
            .unwrap();
        policy
            .set_bot_off("app.other", "carol", "app.notes", true)
            .unwrap();
        policy
            .set_bot_off("app.other", "carol", "app.mail", true)
            .unwrap();
        policy.forget_bot("app.agent", "alice");
        assert_eq!(
            policy.is_bot_off("app.agent", "alice", "app.notes"),
            Ok(false)
        );
        assert_eq!(policy.is_bot_off("app.agent", "bob", "app.notes"), Ok(true));
        policy.forget_plugin("app.agent");
        assert_eq!(
            policy.is_bot_off("app.agent", "bob", "app.notes"),
            Ok(false)
        );
        assert_eq!(
            policy.is_bot_off("app.other", "carol", "app.notes"),
            Ok(true)
        );
        policy.forget_plugin("app.notes");
        assert_eq!(
            policy.is_bot_off("app.other", "carol", "app.notes"),
            Ok(false)
        );
        assert_eq!(
            policy.is_bot_off("app.other", "carol", "app.mail"),
            Ok(true)
        );
    }

    #[test]
    fn an_unreadable_file_does_not_pretend_a_bot_is_switched_on() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.json");
        std::fs::write(&path, "不是 JSON").unwrap();
        let policy = ToolPolicy::load(path);
        assert!(
            policy
                .is_bot_off("app.agent", "alice", "app.notes")
                .is_err()
        );
        assert!(
            policy
                .set_bot_off("app.agent", "alice", "app.notes", true)
                .is_err()
        );
    }
}
