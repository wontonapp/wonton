//! 敏感操作的确认（ADR-0016）：核心发卡片、等主人回答、验证回答，Agent 只能发起，没法替主人同意。
//!
//! 主人的回答只经 [`Confirmations::answer`] 进来，网关只在已配对设备或主人令牌的请求上调用它（ADR-0024）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::future::BoxFuture;
use tokio::sync::oneshot;
use wonton_protocol::{Answer, CardChoice, ConfirmationCard, ConfirmationState};

use super::cards::Draft;
use crate::plugin::PluginId;
use crate::secret::random_token;

/// 同一个插件同时等待确认的请求最多这么多，再多的直接拒绝。
const MAX_PENDING_PER_CALLER: usize = 3;

/// 回答过、过期的确认留这么久，之后再回答得到“不存在”。
const KEEP_FINISHED: Duration = Duration::from_secs(24 * 3600);

/// 把卡片交给主人：发出去，状态变了再更新。
pub trait Notifier: Send + Sync {
    /// 发一张新卡片，返回它的标识，之后用来更新。
    fn post<'a>(&'a self, card: &'a ConfirmationCard) -> BoxFuture<'a, Result<String, String>>;
    /// 更新卡片。原来那张改不了、重发了一张时返回新的标识，之后的更新都用它。
    fn update<'a>(
        &'a self,
        handle: &'a str,
        card: &'a ConfirmationCard,
    ) -> BoxFuture<'a, Result<Option<String>, String>>;
}

/// 没能执行的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Rejected,
    Expired,
    /// 这个插件已经有太多请求在等主人。
    TooMany,
    /// 卡片发不出去，没法问主人。
    Unavailable(String),
}

impl Refusal {
    pub fn message(&self) -> String {
        match self {
            Self::Rejected => "主人拒绝了，没有执行".to_owned(),
            Self::Expired => "主人没有在期限内确认，已经过期，没有执行".to_owned(),
            Self::TooMany => {
                format!("已经有 {MAX_PENDING_PER_CALLER} 个请求在等主人确认，等主人处理完再试")
            }
            Self::Unavailable(e) => format!("没能请主人确认：{e}"),
        }
    }
}

/// 回答失败的原因，网关按它给出 HTTP 状态。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnswerError {
    #[error("没有这个确认")]
    NotFound,
    #[error("这个确认已经回答过了")]
    AlreadyAnswered,
    #[error("这个确认已经过期")]
    Expired,
    #[error("不认识这个回答")]
    UnknownAnswer,
}

impl AnswerError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::AlreadyAnswered => "already_answered",
            Self::Expired => "expired",
            Self::UnknownAnswer => "invalid_input",
        }
    }
}

/// 主人确认之后、执行完之前：执行完用它更新卡片。
pub struct Ticket {
    id: String,
    /// 主人按的是哪个“同意”。
    pub answer: Answer,
}

/// 主人按的那个按钮。
pub struct Chosen {
    pub answer: Answer,
    pub choice_id: Option<String>,
}

struct Slot {
    caller: PluginId,
    card: ConfirmationCard,
    handle: Option<String>,
    waiter: Option<oneshot::Sender<Chosen>>,
    created: SystemTime,
}

pub struct Confirmations {
    notifier: Arc<dyn Notifier>,
    ttl: Duration,
    slots: Mutex<HashMap<String, Slot>>,
}

fn now_ms(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

impl Confirmations {
    /// `ttl`：卡片发出后多久没回答就过期。
    pub fn new(notifier: Arc<dyn Notifier>, ttl: Duration) -> Self {
        Self {
            notifier,
            ttl,
            slots: Mutex::new(HashMap::new()),
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Slot>> {
        self.slots.lock().expect("确认表锁中毒")
    }

    /// 请主人确认，等到主人回答或者过期。确认了返回 [`Ticket`]，执行完要调 [`Self::finish`]。
    pub async fn ask(&self, caller: &PluginId, draft: Draft) -> Result<Ticket, Refusal> {
        let (card, answered) = self.open(caller, draft)?;
        let chosen = self.deliver(&card, answered).await?;
        self.publish(&card.id).await;
        match chosen.answer {
            answer @ (Answer::Approve | Answer::ApproveWider) => Ok(Ticket {
                id: card.id,
                answer,
            }),
            // 不认识的回答到不了这里（`answer` 挡掉了）；真到了也不算同意。
            Answer::Reject | Answer::Unknown => Err(Refusal::Rejected),
        }
    }

    /// 登记一张等回答的卡片，还没发出去。
    pub(super) fn open(
        &self,
        caller: &PluginId,
        draft: Draft,
    ) -> Result<(ConfirmationCard, oneshot::Receiver<Chosen>), Refusal> {
        let created = SystemTime::now();
        let card = ConfirmationCard {
            id: random_token(),
            state: ConfirmationState::Pending,
            title: draft.title,
            lines: draft.lines,
            choices: draft.choices,
            expires_at_ms: now_ms(created + self.ttl),
        };
        let (waiter, answered) = oneshot::channel();
        let mut slots = self.lock();
        slots.retain(|_, slot| {
            slot.waiter.is_some()
                || created.duration_since(slot.created).unwrap_or_default() < KEEP_FINISHED
        });
        let pending = slots
            .values()
            .filter(|s| s.waiter.is_some() && &s.caller == caller)
            .count();
        if pending >= MAX_PENDING_PER_CALLER {
            return Err(Refusal::TooMany);
        }
        let slot = Slot {
            caller: caller.clone(),
            card: card.clone(),
            handle: None,
            waiter: Some(waiter),
            created,
        };
        slots.insert(card.id.clone(), slot);
        Ok((card, answered))
    }

    /// 把 [`Self::open`] 登记的卡片发给主人，等回答。回答之后卡片的变化由调用方发布。
    pub(super) async fn deliver(
        &self,
        card: &ConfirmationCard,
        answered: oneshot::Receiver<Chosen>,
    ) -> Result<Chosen, Refusal> {
        let handle = match self.notifier.post(card).await {
            Ok(handle) => handle,
            Err(e) => {
                self.lock().remove(&card.id);
                return Err(Refusal::Unavailable(e));
            }
        };
        if let Some(slot) = self.lock().get_mut(&card.id) {
            slot.handle = Some(handle);
        }
        match tokio::time::timeout(self.ttl, answered).await {
            Ok(Ok(chosen)) => Ok(chosen),
            Err(_) | Ok(Err(_)) => {
                self.settle(&card.id, ConfirmationState::Expired).await;
                Err(Refusal::Expired)
            }
        }
    }

    /// 等回答的卡片，最早到期的在前。
    pub fn pending(&self) -> Vec<ConfirmationCard> {
        let now = now_ms(SystemTime::now());
        let mut cards: Vec<_> = self
            .lock()
            .values()
            .filter(|slot| slot.waiter.is_some() && slot.card.expires_at_ms > now)
            .map(|slot| slot.card.clone())
            .collect();
        cards.sort_by_key(|card| card.expires_at_ms);
        cards
    }

    /// 主人的回答。确认后卡片变成“正在执行”，拒绝后变成“已拒绝”。
    pub fn answer(
        &self,
        id: &str,
        answer: Answer,
        choice_id: Option<&str>,
    ) -> Result<ConfirmationState, AnswerError> {
        let state = match answer {
            Answer::Approve | Answer::ApproveWider => ConfirmationState::Running,
            Answer::Reject => ConfirmationState::Rejected,
            Answer::Unknown => return Err(AnswerError::UnknownAnswer),
        };
        let mut slots = self.lock();
        let slot = slots.get_mut(id).ok_or(AnswerError::NotFound)?;
        // 卡片上没有这个按钮，就不是对这张卡片的回答。
        let on_card = |c: &CardChoice| c.answer == answer && c.choice_id.as_deref() == choice_id;
        if !slot.card.choices.iter().any(on_card) {
            return Err(AnswerError::UnknownAnswer);
        }
        if slot.card.state == ConfirmationState::Expired
            || now_ms(SystemTime::now()) >= slot.card.expires_at_ms
        {
            return Err(AnswerError::Expired);
        }
        let Some(waiter) = slot.waiter.take() else {
            return Err(AnswerError::AlreadyAnswered);
        };
        // 等待的一方已经不在（只会在核心退出时发生）：当作过期。
        let chosen = Chosen {
            answer,
            choice_id: choice_id.map(str::to_owned),
        };
        if waiter.send(chosen).is_err() {
            slot.card.state = ConfirmationState::Expired;
            return Err(AnswerError::Expired);
        }
        slot.card.state = state.clone();
        Ok(state)
    }

    /// 执行完：卡片更新成结果。
    pub async fn finish(&self, ticket: Ticket, state: ConfirmationState) {
        self.settle(&ticket.id, state).await;
    }

    pub(super) async fn settle(&self, id: &str, state: ConfirmationState) {
        if let Some(slot) = self.lock().get_mut(id) {
            slot.waiter = None;
            slot.card.state = state;
        }
        self.publish(id).await;
    }

    /// 把卡片现在的状态发给主人。更新失败只记日志：执行已经发生了，工具的结果照样返回给 Agent。
    pub(super) async fn publish(&self, id: &str) {
        let current = self
            .lock()
            .get(id)
            .and_then(|slot| Some((slot.handle.clone()?, slot.card.clone())));
        let Some((handle, card)) = current else {
            return;
        };
        match self.notifier.update(&handle, &card).await {
            Ok(None) => {}
            Ok(Some(moved)) => {
                tracing::warn!(confirmation = %id, "原房间不能用了，确认卡片重发到了当前的房间");
                if let Some(slot) = self.lock().get_mut(id) {
                    slot.handle = Some(moved);
                }
            }
            Err(e) => tracing::error!(confirmation = %id, error = %e, "更新确认卡片失败"),
        }
    }
}
