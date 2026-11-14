//! 放行时段（ADR-0024）：卡片上点“N 分钟内都允许”后，这个 Bot 调这个插件到期前不再发卡片。判定只看内存，核心重启
//! 就回到逐次确认；文件只记着哪些卡片还显示着时段，重启后把它们改成已结束。

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use wonton_protocol::{
    Answer, ConfirmationCard, ConfirmationCategory, ConfirmationState, ConfirmationWindow,
    SecretValues, WindowEnd,
};

use super::{Chosen, Confirmations, Slot, Table, now_ms};
use crate::l10n;
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;
use crate::plugin::store::write_json;

/// 还显示着时段的一张卡片。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Persisted {
    caller: String,
    plugin: String,
    /// 按收件人放宽（`wonton__send_message`）的时段记着收件人；插件的时段没有。
    #[serde(default)]
    recipient: Option<String>,
    card: ConfirmationCard,
}

/// 结束了的时段，写审计用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedWindow {
    pub card: String,
    pub caller: String,
    pub plugin: String,
    pub allowed: Option<u32>,
    pub reason: WindowEnd,
}

/// 刚打开的时段，写审计用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedWindow {
    pub caller: String,
    pub plugin: String,
    pub released: u32,
}

/// 生效中的一个时段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveWindow {
    pub card: String,
    pub caller: PluginId,
    pub expires_at_ms: u64,
}

fn load(path: &PathBuf) -> HashMap<String, Persisted> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => return HashMap::new(),
        Err(e) => {
            tracing::error!(path = %path.display(), error = %e, "can't read the saved confirmation windows");
            return HashMap::new();
        }
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        tracing::error!(path = %path.display(), error = %e, "saved confirmation windows are corrupt");
        HashMap::new()
    })
}

impl Confirmations {
    /// 时段多长，以及记着卡片的文件。文件里上次运行留下的卡片用 [`Self::recover_windows`] 改掉。
    #[must_use]
    pub fn with_windows(mut self, length: Duration, file: PathBuf) -> Self {
        self.window = length;
        self.lock().leftovers = load(&file);
        self.windows_file = Some(file);
        self
    }

    pub fn window_length(&self) -> Duration {
        self.window
    }

    /// 只改时长，不记文件。
    #[cfg(test)]
    #[must_use]
    pub fn with_window_length(mut self, length: Duration) -> Self {
        self.window = length;
        self
    }

    /// 这次调用在放行时段内就算一次，返回打开时段的卡片。是否有效在把调用发给插件之前判定，“停止”只拦后面的调用。
    /// `scope` 是 [`super::risky::window_scope`] 给出的范围：插件加 Bot，同一个插件里各 Bot 的时段互不相干。
    pub fn allow_in_window(&self, caller: &PluginId, scope: &str) -> Option<String> {
        let now = now_ms(SystemTime::now());
        let mut table = self.lock();
        let id = table
            .windows
            .get(&(caller.clone(), scope.to_owned()))?
            .clone();
        let window = table.slots.get_mut(&id)?.card.window.as_mut()?;
        if window.ended.is_some() || now >= window.expires_at_ms {
            return None;
        }
        window.allowed = window.allowed.map(|n| n.saturating_add(1));
        Some(id)
    }

    /// 按收件人放宽（ADR-0024，`wonton__send_message` 用）：这个 Bot 给这个收件人发消息的时段
    /// 生效中就算一次，返回打开时段的卡片。时段和插件的互不相干。
    pub fn allow_in_recipient_window(&self, caller: &PluginId, recipient: &str) -> Option<String> {
        let now = now_ms(SystemTime::now());
        let mut table = self.lock();
        let id = table
            .recipient_windows
            .get(&(caller.clone(), recipient.to_owned()))?
            .clone();
        let window = table.slots.get_mut(&id)?.card.window.as_mut()?;
        if window.ended.is_some() || now >= window.expires_at_ms {
            return None;
        }
        window.allowed = window.allowed.map(|n| n.saturating_add(1));
        Some(id)
    }

    /// 这个插件生效中的时段：不带 Bot 的和各 Bot 的都算（时段的范围带 Bot，见 [`super::risky::window_scope`]）。
    pub fn active_windows(&self, plugin: &str) -> Vec<ActiveWindow> {
        let now = now_ms(SystemTime::now());
        let bot_scope = format!("{plugin}#");
        let table = self.lock();
        let active = table
            .windows
            .iter()
            .filter(|((_, scope), _)| scope == plugin || scope.starts_with(&bot_scope));
        let active = active.filter_map(|((caller, _), id)| {
            let window = table.slots.get(id)?.card.window.as_ref()?;
            (window.ended.is_none() && now < window.expires_at_ms).then(|| ActiveWindow {
                card: id.clone(),
                caller: caller.clone(),
                expires_at_ms: window.expires_at_ms,
            })
        });
        active.collect()
    }

    /// 时段从点的那一刻开始算。这个 Bot 在等这个插件确认的其他卡片一起放行：反正它们晚一秒发来也会放行。
    pub(super) fn open_window(&self, table: &mut Table, id: &str, now: u64) {
        let Some(opener) = table.slots.get(id) else {
            return;
        };
        let caller = opener.caller.clone();
        let Some(plugin) = opener.scope.clone() else {
            return;
        };
        let mut released = 0;
        for slot in table.slots.values_mut() {
            if slot.caller != caller
                || slot.scope.as_ref() != Some(&plugin)
                || now >= slot.card.expires_at_ms
            {
                continue;
            }
            let Some(waiter) = slot.waiter.take() else {
                continue;
            };
            let chosen = Chosen {
                answer: Answer::Approve,
                choice_id: None,
                secrets: SecretValues::new(),
                settings: Default::default(),
            };
            if waiter.send(chosen).is_ok() {
                slot.card.state = ConfirmationState::Running;
                slot.ended = Some(now);
                slot.released_by = Some(id.to_owned());
                released += 1;
            }
        }
        table.windows.insert((caller, plugin), id.to_owned());
        if let Some(slot) = table.slots.get_mut(id) {
            let length = u64::try_from(self.window.as_millis()).unwrap_or(u64::MAX);
            slot.card.window = Some(ConfirmationWindow {
                expires_at_ms: now.saturating_add(length),
                allowed: Some(released),
                ended: None,
            });
        }
    }

    /// 给收件人发消息的时段从点的那一刻开始算（ADR-0024 按收件人放宽）：这个 Bot 在等对同一个
    /// 收件人发消息的其他卡片一起放行，反正它们晚一秒发来也会放行。
    pub(super) fn open_recipient_window(&self, table: &mut Table, id: &str, now: u64) {
        let Some(opener) = table.slots.get(id) else {
            return;
        };
        let caller = opener.caller.clone();
        let Some(ConfirmationCategory::SendMessage { recipient, .. }) = &opener.card.category
        else {
            return;
        };
        let recipient = recipient.clone();
        let mut released = 0;
        for slot in table.slots.values_mut() {
            if slot.caller != caller || now >= slot.card.expires_at_ms {
                continue;
            }
            let Some(ConfirmationCategory::SendMessage {
                recipient: other, ..
            }) = &slot.card.category
            else {
                continue;
            };
            if other != &recipient {
                continue;
            }
            let Some(waiter) = slot.waiter.take() else {
                continue;
            };
            let chosen = Chosen {
                answer: Answer::Approve,
                choice_id: None,
                secrets: SecretValues::new(),
                settings: Default::default(),
            };
            if waiter.send(chosen).is_ok() {
                slot.card.state = ConfirmationState::Running;
                slot.ended = Some(now);
                slot.released_by = Some(id.to_owned());
                released += 1;
            }
        }
        table
            .recipient_windows
            .insert((caller, recipient), id.to_owned());
        if let Some(slot) = table.slots.get_mut(id) {
            let length = u64::try_from(self.window.as_millis()).unwrap_or(u64::MAX);
            slot.card.window = Some(ConfirmationWindow {
                expires_at_ms: now.saturating_add(length),
                allowed: Some(released),
                ended: None,
            });
        }
    }

    /// 这张卡片打开的时段：刚打开时写审计用。
    pub fn opened_window(&self, id: &str) -> Option<OpenedWindow> {
        let table = self.lock();
        let slot = table.slots.get(id)?;
        Some(OpenedWindow {
            caller: slot.caller.to_string(),
            plugin: slot.scope.clone()?,
            released: slot.card.window.as_ref()?.allowed?,
        })
    }

    /// 结束时段，把卡片改成已结束。没有这个时段或者已经结束时返回 `None`。
    pub async fn end_window(&self, id: &str, reason: WindowEnd) -> Option<EndedWindow> {
        let ended = {
            let mut table = self.lock();
            let slot = table.slots.get_mut(id)?;
            let window = slot.card.window.as_mut()?;
            if window.ended.is_some() {
                return None;
            }
            window.ended = Some(reason);
            let ended = EndedWindow {
                card: id.to_owned(),
                caller: slot.caller.to_string(),
                plugin: slot.scope.clone().unwrap_or_default(),
                allowed: window.allowed,
                reason,
            };
            table.windows.retain(|_, card| card != id);
            ended
        };
        self.bump();
        self.save_windows();
        Some(ended)
    }

    /// 文件里是还显示着时段的卡片。写不进去只记错误日志，时段照常：这个文件只管改旧卡片。
    pub(super) fn save_windows(&self) {
        let Some(file) = &self.windows_file else {
            return;
        };
        let table = self.lock();
        let mut saved = table.leftovers.clone();
        // 按收件人的时段：卡片 ID 反查收件人。两种时段互斥，一张卡片只有一种。
        let recipients: HashMap<&str, &String> = table
            .recipient_windows
            .iter()
            .map(|((_, recipient), id)| (id.as_str(), recipient))
            .collect();
        for (id, slot) in &table.slots {
            let open = slot.card.window.as_ref().is_some_and(|w| w.ended.is_none());
            if !open {
                continue;
            }
            let recipient = recipients.get(id.as_str()).map(|r| (*r).clone());
            let plugin = match (&slot.scope, &recipient) {
                (Some(plugin), _) => plugin.clone(),
                (None, Some(_)) => String::new(),
                (None, None) => continue,
            };
            let entry = Persisted {
                caller: slot.caller.to_string(),
                plugin,
                recipient,
                card: slot.card.clone(),
            };
            saved.insert(id.clone(), entry);
        }
        if let Err(e) = write_json(file, &saved) {
            tracing::error!(path = %file.display(), error = %e, "can't save the confirmation windows");
        }
    }

    pub fn has_leftover_windows(&self) -> bool {
        !self.lock().leftovers.is_empty()
    }

    /// 把上次运行留下的卡片改成“核心重启，已结束”，写回确认表：结束的卡片只在确认表里，App 从列表看到
    /// “结果不确定”。改成功的才从文件里删，没成功的下次再试。
    pub async fn recover_windows(&self, langs: &Languages) -> Vec<EndedWindow> {
        let uncertain = l10n::text(langs, "chat-uncertain-restart", &[]);
        let now = now_ms(SystemTime::now());
        let leftovers: Vec<_> = self
            .lock()
            .leftovers
            .iter()
            .map(|(id, entry)| (id.clone(), entry.clone()))
            .collect();
        let mut ended = Vec::new();
        for (id, entry) in leftovers {
            let mut card = entry.card;
            if let Some(window) = card.window.as_mut() {
                window.allowed = None;
                window.ended = Some(WindowEnd::CoreRestarted);
            }
            // 核心停的时候这次调用可能还在执行，结果不知道了。
            if card.state == ConfirmationState::Running {
                card.state = ConfirmationState::Uncertain {
                    message: uncertain.clone(),
                };
            }
            let Ok(caller) = PluginId::parse(&entry.caller) else {
                tracing::error!(confirmation = %id, caller = %entry.caller, "留着的放行时段记着不认识的插件，丢下它");
                self.lock().leftovers.remove(&id);
                continue;
            };
            // 放行时段只有主人的卡片有（家人的卡片不给这个选项），收件人是主人。
            let slot = Slot {
                caller,
                recipients: super::Recipients::Keeper(None),
                scope: Some(entry.plugin.clone()),
                released_by: None,
                card: card.clone(),
                waiter: None,
                secret_inputs: Vec::new(),
                ended: Some(now),
            };
            self.lock().slots.insert(id.clone(), slot);
            self.lock().leftovers.remove(&id);
            ended.push(EndedWindow {
                card: id,
                caller: entry.caller,
                plugin: entry.plugin,
                allowed: None,
                reason: WindowEnd::CoreRestarted,
            });
        }
        if !ended.is_empty() {
            self.bump();
            self.save_windows();
        }
        ended
    }
}
