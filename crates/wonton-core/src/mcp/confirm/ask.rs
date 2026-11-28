//! 开卡：登记一张等回答的卡片，推给收件人的设备，等到回答或过期。

use std::time::{Duration, SystemTime};

use tokio::sync::oneshot;
use wonton_protocol::{Answer, ConfirmationCard, ConfirmationState};

use super::types::{Audience, CANCELLED, MAX_PENDING_PER_CALLER, Recipients, Refusal, SUPERSEDED};
use super::{Chosen, Confirmations, Slot, Ticket, now_ms};
use crate::mcp::cards::Draft;
use crate::plugin::PluginId;
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
    pub(in crate::mcp) async fn wait(
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
    pub(in crate::mcp) fn open_as(
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
    pub(in crate::mcp) async fn deliver(
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
}
