//! 结束等待：执行完、被新提交取代、被核心撤回，以及超时后的收尾。

use std::time::SystemTime;

use super::{CANCELLED, Chosen, Confirmations, Refusal, SUPERSEDED, Ticket, now_ms};
use crate::l10n;
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;
use wonton_protocol::{Answer, ConfirmationState, SecretValues};

impl Confirmations {
    /// 执行完：卡片更新成结果。
    pub async fn finish(&self, ticket: Ticket, state: ConfirmationState) {
        self.settle(&ticket.id, state).await;
        self.save_windows();
    }

    /// 还在等主人的确认被新提交取代（ADR-0033 的暂存）：卡片改成“已被新版本取代”，
    /// 等待的那次工具调用拿到 [`Refusal::Superseded`]。已经不在等（回答了或在执行）时不动，返回假。
    pub(in crate::mcp) async fn supersede(&self, id: &str, langs: &Languages) -> bool {
        let superseded = l10n::text(langs, "chat-superseded", &[]);
        let chosen = self.lock().slots.get_mut(id).and_then(|slot| {
            let waiter = slot.waiter.take()?;
            slot.card.state = ConfirmationState::Failed {
                message: superseded,
            };
            slot.ended = Some(now_ms(SystemTime::now()));
            Some(waiter)
        });
        let Some(waiter) = chosen else {
            return false;
        };
        self.activity_changed();
        let _ = waiter.send(Chosen {
            answer: Answer::Reject,
            choice_id: Some(SUPERSEDED.to_owned()),
            secrets: SecretValues::new(),
            settings: Default::default(),
            devices: Vec::new(),
        });
        self.bump();
        true
    }

    /// 核心撤回了这张卡片（链被停，ADR-0061 第 10 节）：还在等的等待方拿到“已取消”，卡片改成结果。
    /// 已经不在等（回答了、过期了）时不动，返回假。
    pub(in crate::mcp) fn cancel(&self, id: &str, message: &str) -> bool {
        let chosen = self.lock().slots.get_mut(id).and_then(|slot| {
            let waiter = slot.waiter.take()?;
            slot.card.state = ConfirmationState::Failed {
                message: message.to_owned(),
            };
            slot.ended = Some(now_ms(SystemTime::now()));
            Some(waiter)
        });
        let Some(waiter) = chosen else {
            return false;
        };
        self.activity_changed();
        let _ = waiter.send(Chosen {
            answer: Answer::Reject,
            choice_id: Some(CANCELLED.to_owned()),
            secrets: SecretValues::new(),
            settings: Default::default(),
            devices: Vec::new(),
        });
        self.bump();
        true
    }

    /// 这个 Bot 还在等的、关于某个插件的卡片：被这个插件的新提交取代（ADR-0033 的暂存）。
    pub(in crate::mcp) async fn supersede_pending(
        &self,
        caller: &PluginId,
        plugin: &PluginId,
        langs: &Languages,
    ) -> bool {
        let pending = {
            let table = self.lock();
            table
                .slots
                .values()
                .find(|slot| {
                    slot.caller == *caller
                        && slot.waiter.is_some()
                        && slot.card.plugin_id.as_deref() == Some(plugin.as_str())
                })
                .map(|slot| slot.card.id.clone())
        };
        let Some(id) = pending else {
            return false;
        };
        self.supersede(&id, langs).await
    }

    pub(in crate::mcp) async fn settle(&self, id: &str, state: ConfirmationState) {
        if let Some(slot) = self.lock().slots.get_mut(id) {
            if slot.waiter.take().is_some() {
                slot.ended = Some(now_ms(SystemTime::now()));
                self.activity_changed();
            }
            slot.card.state = state;
        }
        self.bump();
    }
}
