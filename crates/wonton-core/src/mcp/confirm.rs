//! 敏感操作的确认（ADR-0016）：核心发卡片、等主人回答、验证回答，Agent 只能发起，没法替主人同意。
//!
//! 主人的回答只经 [`Confirmations::answer`] 进来，网关只在已配对设备或主人令牌的请求上调用它（ADR-0024）。

mod activity;
mod ask;
mod store;
mod types;
mod windows;

pub use windows::EndedWindow;

#[allow(unused_imports)] // 只有测试经 `super::confirm::DEFAULT_WINDOW` 用。
pub use store::{Confirmations, DEFAULT_WINDOW};
pub use types::{AnswerError, Audience, Chosen, Notifier, Recipients, Refusal, Ticket};

pub(super) use store::{Slot, Table, now_ms};
pub(super) use types::{CANCELLED, SUPERSEDED};

use std::time::{Duration, SystemTime};

use tokio::sync::watch;
use wonton_protocol::{Answer, CardChoice, ConfirmationCard, ConfirmationState, SecretValues};

use crate::l10n;
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;

impl Confirmations {
    /// “等你确认”列表（ADR-0016）：等回答的（最早到期的在前）、结束不到保留期的（最近结束的在前），和当前版本。
    /// 只列发给这个人的（`None` 是主人）：家人的列表里没有主人和别人的卡片。
    pub fn list(
        &self,
        recipient: Option<&str>,
    ) -> (Vec<ConfirmationCard>, Vec<ConfirmationCard>, u64) {
        let now = now_ms(SystemTime::now());
        let table = self.lock();
        let mut pending: Vec<&Slot> = Vec::new();
        let mut finished: Vec<&Slot> = Vec::new();
        for slot in table.slots.values() {
            if !slot.includes(recipient) {
                continue;
            }
            if slot.waiter.is_some() && slot.card.expires_at_ms > now {
                pending.push(slot);
            } else if let Some(ended) = slot.ended
                && Duration::from_millis(now.saturating_sub(ended)) < self.keep
            {
                finished.push(slot);
            }
        }
        pending.sort_by_key(|slot| slot.card.expires_at_ms);
        finished.sort_by_key(|slot| std::cmp::Reverse(slot.ended));
        let revision = *self.revision.borrow();
        (
            pending.into_iter().map(|s| s.card.clone()).collect(),
            finished.into_iter().map(|s| s.card.clone()).collect(),
            revision,
        )
    }

    /// 一张卡片：等回答的、和结束不到保留期的都取得到，再早的没有。收件人用 [`Self::recipient`] 查。
    pub fn card(&self, id: &str) -> Option<ConfirmationCard> {
        let now = now_ms(SystemTime::now());
        self.lock().slots.get(id).and_then(|slot| {
            let pending = slot.waiter.is_some() && slot.card.expires_at_ms > now;
            let kept = slot
                .ended
                .is_some_and(|ended| Duration::from_millis(now.saturating_sub(ended)) < self.keep);
            (pending || kept).then(|| slot.card.clone())
        })
    }

    /// 当前列表的版本。
    pub fn revision(&self) -> u64 {
        *self.revision.borrow()
    }

    /// 版本变化的订阅：网关的长轮询挂在它上面等下一张卡片。
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    /// 单收件人卡片返回收件人；管理员卡片和不存在的 ID 返回 `None`。
    pub fn recipient(&self, id: &str) -> Option<Option<String>> {
        self.lock().slots.get(id).and_then(|slot| {
            matches!(&slot.recipients, Recipients::Keeper(_)).then(|| slot.recipients.keeper())
        })
    }

    /// 判断这个人是否属于卡片创建时冻结的收件人集合；不存在的 ID 返回 `None`。
    pub fn may_answer(&self, id: &str, recipient: Option<&str>) -> Option<bool> {
        self.lock()
            .slots
            .get(id)
            .map(|slot| slot.includes(recipient))
    }

    /// 回答接口的权限判断：管理员卡片已有其他人回答时，其他管理员不能重复回答。
    pub fn may_answer_as(
        &self,
        id: &str,
        recipient: Option<&str>,
        user_id: Option<&str>,
    ) -> Option<bool> {
        self.lock().slots.get(id).map(|slot| {
            slot.includes(recipient)
                && slot
                    .card
                    .answered_by
                    .as_deref()
                    .is_none_or(|answered_by| user_id == Some(answered_by))
        })
    }

    /// 主人的回答。确认后卡片变成“正在执行”，拒绝后变成“已拒绝”。
    pub fn answer(
        &self,
        id: &str,
        answer: Answer,
        choice_id: Option<&str>,
    ) -> Result<ConfirmationState, AnswerError> {
        self.answer_with_secrets(
            id,
            answer,
            choice_id,
            SecretValues::new(),
            Default::default(),
            Vec::new(),
            &Languages::from_header(None),
        )
    }

    /// 带密钥的回答（ADR-0064）。
    #[allow(clippy::too_many_arguments)] // Owner answer values are forwarded together without duplicating form state.
    pub fn answer_with_secrets(
        &self,
        id: &str,
        answer: Answer,
        choice_id: Option<&str>,
        secrets: SecretValues,
        settings: wonton_protocol::SettingValues,
        devices: Vec<String>,
        langs: &Languages,
    ) -> Result<ConfirmationState, AnswerError> {
        self.answer_with_secrets_by(
            id, answer, choice_id, secrets, settings, devices, langs, None,
        )
    }

    /// Gateway-authenticated answerer, recorded atomically with the winning answer.
    #[allow(clippy::too_many_arguments)]
    pub fn answer_with_secrets_by(
        &self,
        id: &str,
        answer: Answer,
        choice_id: Option<&str>,
        secrets: SecretValues,
        settings: wonton_protocol::SettingValues,
        devices: Vec<String>,
        langs: &Languages,
        answered_by: Option<&str>,
    ) -> Result<ConfirmationState, AnswerError> {
        let state = match answer {
            Answer::Approve | Answer::ApproveWider | Answer::AllowWindow => {
                ConfirmationState::Running
            }
            Answer::Reject => ConfirmationState::Rejected,
            Answer::Unknown => return Err(AnswerError::UnknownAnswer),
        };
        let now = now_ms(SystemTime::now());
        let mut table = self.lock();
        let slot = table.slots.get_mut(id).ok_or(AnswerError::NotFound)?;
        // 卡片上没有这个按钮，就不是对这张卡片的回答。
        let on_card = |c: &CardChoice| c.answer == answer && c.choice_id.as_deref() == choice_id;
        if !slot.card.choices.iter().any(on_card) {
            return Err(AnswerError::UnknownAnswer);
        }
        if slot.card.state == ConfirmationState::Expired || now >= slot.card.expires_at_ms {
            return Err(AnswerError::Expired);
        }
        // 先查密钥：不对时卡片照常等回答。
        let secrets = if slot.secret_inputs.is_empty() {
            if !secrets.is_empty() {
                return Err(AnswerError::Secrets(crate::l10n::text(
                    langs,
                    "chat-card-secrets-not-used",
                    &[],
                )));
            }
            secrets
        } else {
            if let Some(name) = secrets
                .keys()
                .find(|name| !slot.secret_inputs.iter().any(|i| &i.name == *name))
            {
                return Err(AnswerError::Secrets(crate::l10n::text(
                    langs,
                    "plugin-secret-unknown",
                    &[("names", crate::l10n::list(langs, &[name.as_str()]).into())],
                )));
            }
            // 必填项没填也能回答：插件照样装，状态是“待配置”（ADR-0064）。
            secrets
        };
        let Some(waiter) = slot.waiter.take() else {
            return Err(AnswerError::AlreadyAnswered);
        };
        // 等待的一方已经不在（只会在核心退出时发生）：当作过期。
        let chosen = Chosen {
            answer,
            choice_id: choice_id.map(str::to_owned),
            secrets,
            settings,
            devices,
        };
        if waiter.send(chosen).is_err() {
            slot.card.state = ConfirmationState::Expired;
            slot.ended = Some(now);
            self.activity_changed();
            self.bump();
            return Err(AnswerError::Expired);
        }
        slot.card.state = state.clone();
        slot.card.answered_by = answered_by.map(str::to_owned);
        slot.ended = Some(now);
        if answer == Answer::AllowWindow {
            // 发消息的卡片按收件人放宽，其余按插件。
            let send_message = matches!(
                slot.card.category,
                Some(wonton_protocol::ConfirmationCategory::SendMessage { .. })
            );
            if send_message {
                self.open_recipient_window(&mut table, id, now);
            } else {
                self.open_window(&mut table, id, now);
            }
        }
        self.activity_changed();
        self.bump();
        Ok(state)
    }

    /// 执行完：卡片更新成结果。
    pub async fn finish(&self, ticket: Ticket, state: ConfirmationState) {
        self.settle(&ticket.id, state).await;
        self.save_windows();
    }

    /// 还在等主人的确认被新提交取代（ADR-0033 的暂存）：卡片改成“已被新版本取代”，
    /// 等待的那次工具调用拿到 [`Refusal::Superseded`]。已经不在等（回答了或在执行）时不动，返回假。
    pub(super) async fn supersede(&self, id: &str, langs: &Languages) -> bool {
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
    pub(super) fn cancel(&self, id: &str, message: &str) -> bool {
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
    pub(super) async fn supersede_pending(
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

    pub(super) async fn settle(&self, id: &str, state: ConfirmationState) {
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
