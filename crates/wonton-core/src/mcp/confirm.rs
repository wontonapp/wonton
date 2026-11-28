//! 敏感操作的确认（ADR-0016）：核心发卡片、等主人回答、验证回答，Agent 只能发起，没法替主人同意。
//!
//! 主人的回答只经 [`Confirmations::answer`] 进来，网关只在已配对设备或主人令牌的请求上调用它（ADR-0024）。

mod activity;
mod store;
mod types;
mod windows;

pub use windows::EndedWindow;

#[allow(unused_imports)] // 只有测试经 `super::confirm::DEFAULT_WINDOW` 用。
pub use store::{Confirmations, DEFAULT_WINDOW};
pub use types::{AnswerError, Audience, Chosen, Notifier, Recipients, Refusal, Ticket};

pub(super) use types::{CANCELLED, MAX_PENDING_PER_CALLER, SUPERSEDED};

pub(super) use store::{Slot, Table, now_ms};

use std::time::{Duration, SystemTime};

use tokio::sync::{oneshot, watch};
use wonton_protocol::{Answer, CardChoice, ConfirmationCard, ConfirmationState, SecretValues};

use super::cards::Draft;
use crate::l10n;
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;
use crate::secret::random_token;

impl Confirmations {
    /// 请主人确认，等到主人回答或者过期。确认了返回 [`Ticket`]，执行完要调 [`Self::finish`]。
    pub async fn ask(&self, caller: &PluginId, draft: Draft) -> Result<Ticket, Refusal> {
        self.ask_as(caller, draft, Audience::default()).await
    }

    /// 同 [`Self::ask`]，卡片发给 `audience` 的收件人、带上发起者和来源会话（ADR-0016）。
    pub async fn ask_as(
        &self,
        caller: &PluginId,
        draft: Draft,
        audience: Audience,
    ) -> Result<Ticket, Refusal> {
        self.ask_audience(caller, draft, audience).await
    }

    async fn ask_audience(
        &self,
        caller: &PluginId,
        draft: Draft,
        audience: Audience,
    ) -> Result<Ticket, Refusal> {
        let (card, answered) = self.open_audience(caller, draft, audience)?;
        self.wait(card, answered).await
    }

    /// 等一张已经开出来的卡片的回答，把回答变成 [`Ticket`]。给要先绑链、再等回答的调用方用。
    pub(super) async fn wait(
        &self,
        card: ConfirmationCard,
        answered: oneshot::Receiver<Chosen>,
    ) -> Result<Ticket, Refusal> {
        let chosen = self.deliver(&card, answered).await?;
        match chosen.answer {
            answer @ (Answer::Approve | Answer::ApproveWider | Answer::AllowWindow) => {
                if answer == Answer::AllowWindow {
                    self.save_windows();
                }
                let window = self
                    .lock()
                    .slots
                    .get(&card.id)
                    .and_then(|slot| slot.released_by.clone());
                Ok(Ticket {
                    id: card.id,
                    answer,
                    secrets: chosen.secrets,
                    settings: chosen.settings,
                    devices: chosen.devices,
                    window,
                })
            }
            // [`Self::supersede`] 让这次等待结束：不是主人的回答，错误要能和拒绝分清。
            Answer::Reject if chosen.choice_id.as_deref() == Some(SUPERSEDED) => {
                Err(Refusal::Superseded)
            }
            Answer::Reject if chosen.choice_id.as_deref() == Some(CANCELLED) => {
                Err(Refusal::Cancelled)
            }
            // 不认识的回答到不了这里（`answer` 挡掉了）；真到了也不算同意。
            // 拒绝时 `deliver` 已经发布过卡片状态。
            Answer::Reject | Answer::Unknown => Err(Refusal::Rejected),
        }
    }

    /// 登记一张等回答的卡片，还没发出去。
    pub(super) fn open_as(
        &self,
        caller: &PluginId,
        draft: Draft,
        audience: Audience,
    ) -> Result<(ConfirmationCard, oneshot::Receiver<Chosen>), Refusal> {
        self.open_audience(caller, draft, audience)
    }

    fn open_audience(
        &self,
        caller: &PluginId,
        draft: Draft,
        audience: Audience,
    ) -> Result<(ConfirmationCard, oneshot::Receiver<Chosen>), Refusal> {
        let created = SystemTime::now();
        let Audience {
            recipients,
            requester,
            on_behalf,
            origin,
            category,
        } = audience;
        let card = ConfirmationCard {
            id: random_token(),
            state: ConfirmationState::Pending,
            title: draft.title,
            lines: draft.lines,
            choices: draft.choices,
            expires_at_ms: now_ms(created + self.ttl),
            plugin_id: draft.plugin_id,
            secret_inputs: draft.secret_inputs.clone(),
            settings: draft.settings.clone(),
            permissions: draft.permissions,
            devices: draft.devices,
            window: None,
            requester,
            on_behalf,
            origin,
            audience: match &recipients {
                Recipients::Keeper(_) => wonton_protocol::CardAudience::Keeper,
                Recipients::Admins(_) => wonton_protocol::CardAudience::Admins,
            },
            answered_by: None,
            category: category.or(draft.category),
        };
        let (waiter, answered) = oneshot::channel();
        let mut table = self.lock();
        self.prune(&mut table, created);
        let pending = table
            .slots
            .values()
            .filter(|s| s.waiter.is_some() && &s.caller == caller)
            .count();
        if pending >= MAX_PENDING_PER_CALLER {
            return Err(Refusal::TooMany);
        }
        let slot = Slot {
            caller: caller.clone(),
            recipients,
            scope: draft.scope,
            released_by: None,
            card: card.clone(),
            waiter: Some(waiter),
            secret_inputs: draft.secret_inputs,
            ended: None,
        };
        table.slots.insert(card.id.clone(), slot);
        self.activity_changed();
        self.bump();
        Ok((card, answered))
    }

    /// 通知收件人的设备有一张新卡片，然后等回答。回答之后卡片的变化只改确认表。
    pub(super) async fn deliver(
        &self,
        card: &ConfirmationCard,
        answered: oneshot::Receiver<Chosen>,
    ) -> Result<Chosen, Refusal> {
        let expires =
            Duration::from_millis(card.expires_at_ms.saturating_sub(now_ms(SystemTime::now())));
        let deadline = tokio::time::Instant::now() + expires;
        let recipients = {
            let table = self.lock();
            table
                .slots
                .get(&card.id)
                .map(|slot| slot.recipients.clone())
                .ok_or(Refusal::Expired)?
        };
        let posted = self.notifier.post(&recipients, card);
        match tokio::time::timeout_at(deadline, posted).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                self.lock().slots.remove(&card.id);
                self.activity_changed();
                self.bump();
                return Err(Refusal::Unavailable(e));
            }
            Err(_) => {
                self.settle(&card.id, ConfirmationState::Expired).await;
                return Err(Refusal::Expired);
            }
        }
        match tokio::time::timeout_at(deadline, answered).await {
            Ok(Ok(chosen)) => Ok(chosen),
            Err(_) | Ok(Err(_)) => {
                self.settle(&card.id, ConfirmationState::Expired).await;
                Err(Refusal::Expired)
            }
        }
    }

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
