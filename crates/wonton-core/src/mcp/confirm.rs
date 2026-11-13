//! 敏感操作的确认（ADR-0016）：核心发卡片、等主人回答、验证回答，Agent 只能发起，没法替主人同意。
//!
//! 主人的回答只经 [`Confirmations::answer`] 进来，网关只在已配对设备或主人令牌的请求上调用它（ADR-0024）。

mod activity;
mod windows;

pub use windows::EndedWindow;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::future::BoxFuture;
use tokio::sync::{oneshot, watch};
use wonton_protocol::{
    Answer, CardChoice, ConfirmationCard, ConfirmationOrigin, ConfirmationRequester,
    ConfirmationState, SecretInput, SecretValues,
};

use super::cards::Draft;
use crate::l10n;
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;
use crate::secret::random_token;

/// 同一个插件同时等待确认的请求最多这么多，再多的直接拒绝。
const MAX_PENDING_PER_CALLER: usize = 3;

/// [`Self::supersede`] 送给等待方的标记：这次等待结束是因为被新提交取代，不是主人拒绝。
const SUPERSEDED: &str = "superseded";

/// [`Self::cancel`] 送给等待方的标记：卡片被核心撤回了（链被停），不是主人拒绝，也不是过期。
pub(super) const CANCELLED: &str = "cancelled";

/// 回答过、过期的确认留这么久，之后再回答得到“不存在”。
const KEEP_FINISHED: Duration = Duration::from_secs(24 * 3600);

/// 放行时段默认多长（`WONTON_CONFIRMATION_WINDOW_SECS`，ADR-0024）。
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(30 * 60);

/// 把新卡片通知给收件人的设备（ADR-0016 的“推送”）：开卡时发一条推送，之后状态变化只改确认表，
/// 设备从 [`CONFIRMATIONS_PATH`](wonton_protocol::CONFIRMATIONS_PATH) 的长轮询里知道。卡片不进任何 Matrix 房间。
pub trait Notifier: Send + Sync {
    /// 通知 `recipient`（家人的用户名；`None` 是主人）的设备有一张新卡片。他名下没有已配对的设备时报错
    /// （没人能答，调用得到 `confirmation_unavailable`）；推送服务暂时不可用不报错，卡片已在确认表里，
    /// App 照常看得到。
    fn post<'a>(
        &'a self,
        recipient: Option<&'a str>,
        card: &'a ConfirmationCard,
    ) -> BoxFuture<'a, Result<(), String>>;
}

/// 卡片发给谁、谁发起的、显示在哪个会话。默认（全空）是发给主人、没有发起者和来源会话。
#[derive(Clone, Debug, Default)]
pub struct Audience {
    /// 回答这张卡片的家人的用户名；`None` 是主人。只有这个人能在列表里看到、能回答。
    pub recipient: Option<String>,
    pub requester: Option<ConfirmationRequester>,
    pub origin: Option<ConfirmationOrigin>,
}

/// 没能执行的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Rejected,
    Expired,
    /// 同一个（Bot，插件）有了新的确认请求，旧的这份被取代（ADR-0033 的暂存），等新的那份。
    Superseded,
    /// 核心撤回了卡片（链被停，ADR-0061 第 10 节）。
    Cancelled,
    /// 这个插件已经有太多请求在等主人。
    TooMany,
    /// 卡片发不出去，没法问主人。
    Unavailable(String),
}

impl Refusal {
    pub fn message(&self) -> String {
        match self {
            Self::Rejected => "你拒绝了，没有执行".to_owned(),
            Self::Expired => "你没有在期限内确认，已经过期，没有执行".to_owned(),
            Self::Superseded => {
                "这个插件有了新的确认请求，这一份已经被取代，只有最新的那份能确认".to_owned()
            }
            Self::Cancelled => "这条链被停了，卡片随之撤回".to_owned(),
            Self::TooMany => {
                format!("已经有 {MAX_PENDING_PER_CALLER} 个请求在等你确认，等你处理完再试")
            }
            Self::Unavailable(e) => format!("没能请你确认：{e}"),
        }
    }

    /// 结果 `_meta` 的 `app.wonton/failure`：Agent 和 App 不用读文案就知道是哪种没执行。
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Rejected => "owner_rejected",
            Self::Expired => "confirmation_expired",
            Self::Superseded => "superseded",
            Self::Cancelled => "chain_stopped",
            Self::TooMany => "too_many_pending",
            Self::Unavailable(_) => "confirmation_unavailable",
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
    /// 卡片要填的密钥不对：多出不认识的项，或者这张卡片不收密钥。卡片照常等回答；必填项没填不算错。
    #[error("{0}")]
    Secrets(String),
}

impl AnswerError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::AlreadyAnswered => "already_answered",
            Self::Expired => "expired",
            Self::UnknownAnswer | Self::Secrets(_) => "invalid_input",
        }
    }
}

/// 主人确认之后、执行完之前：执行完用它更新卡片。
pub struct Ticket {
    id: String,
    /// 主人按的是哪个“同意”。
    pub answer: Answer,
    /// 主人在卡片里填的密钥（ADR-0064）。
    pub secrets: SecretValues,
    /// 主人在卡片里填的设置（ADR-0077）。
    pub settings: wonton_protocol::SettingValues,
    /// 主人没回答这张卡片，是另一张卡片打开放行时段时一起放行的：那张卡片的 ID。
    pub window: Option<String>,
}

/// 主人按的那个按钮。
pub struct Chosen {
    pub answer: Answer,
    pub choice_id: Option<String>,
    /// 主人在卡片里填的密钥（ADR-0064）；别的卡片是空表。
    pub secrets: SecretValues,
    /// 主人在卡片里填的设置（ADR-0077）；别的卡片是空表。
    pub settings: wonton_protocol::SettingValues,
}

struct Slot {
    caller: PluginId,
    /// 见 [`Audience::recipient`]。
    recipient: Option<String>,
    /// 这张卡片能打开放行时段时，时段管哪个插件。
    scope: Option<String>,
    /// 见 [`Ticket::window`]。
    released_by: Option<String>,
    card: ConfirmationCard,
    waiter: Option<oneshot::Sender<Chosen>>,
    /// 这张卡片要填的密钥（ADR-0064）；不填密钥的卡片是空的。
    secret_inputs: Vec<SecretInput>,
    /// 不再等回答的时刻（Unix 毫秒）：答了、拒了、过期或被取代。结束的卡片按 [`KEEP_FINISHED`] 留这么久。
    ended: Option<u64>,
}

#[derive(Default)]
struct Table {
    slots: HashMap<String, Slot>,
    /// 生效中的放行时段：（Bot，插件）→ 打开它的卡片。其余状态在卡片的 `window` 里。
    windows: HashMap<(PluginId, String), String>,
    /// 按收件人放宽的时段（ADR-0024，`wonton__send_message`）：（Bot，收件人）→ 打开它的卡片。
    recipient_windows: HashMap<(PluginId, String), String>,
    /// 上次运行留下、卡片还没改成“已结束”的时段。
    leftovers: HashMap<String, windows::Persisted>,
}

pub struct Confirmations {
    notifier: Arc<dyn Notifier>,
    ttl: Duration,
    window: Duration,
    /// 结束的卡片留多久（`KEEP_FINISHED`，测试里可改短）。
    keep: Duration,
    /// 记着哪些卡片显示着时段，核心重启后才改得了它们。判定不读它。
    windows_file: Option<PathBuf>,
    table: Mutex<Table>,
    /// 列表的版本：任何一张卡片变了就加一，网关的长轮询挂在这个通道上（[`wonton_protocol::CONFIRMATIONS_PATH`]）。
    revision: watch::Sender<u64>,
    /// 确认状态变化要唤醒的活动表。挂它的 Bus 可能重建（测试里共用 Confirmations），
    /// 所以允许换接：新 Bus 的活动表才是当前事实源。
    activity: RwLock<Option<Arc<crate::bot_activity::BotActivity>>>,
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
            window: DEFAULT_WINDOW,
            keep: KEEP_FINISHED,
            windows_file: None,
            table: Mutex::new(Table::default()),
            revision: watch::Sender::new(0),
            activity: RwLock::new(None),
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// 结束的卡片留多久（测试里改短，好验证“过了保留期取不到”）。
    #[cfg(test)]
    #[must_use]
    pub fn with_keep_finished(mut self, keep: Duration) -> Self {
        self.keep = keep;
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Table> {
        self.table.lock().expect("确认表锁中毒")
    }

    /// 请主人确认，等到主人回答或者过期。确认了返回 [`Ticket`]，执行完要调 [`Self::finish`]。
    pub async fn ask(&self, caller: &PluginId, draft: Draft) -> Result<Ticket, Refusal> {
        self.ask_as(caller, draft, Audience::default()).await
    }

    /// 同 [`Self::ask`]，卡片发给 `audience` 说的人、带上发起者和来源会话（ADR-0016）。
    pub async fn ask_as(
        &self,
        caller: &PluginId,
        draft: Draft,
        audience: Audience,
    ) -> Result<Ticket, Refusal> {
        let (card, answered) = self.open_as(caller, draft, audience)?;
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
        let created = SystemTime::now();
        let Audience {
            recipient,
            requester,
            origin,
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
            window: None,
            requester,
            origin,
            audience: wonton_protocol::CardAudience::Keeper,
            answered_by: None,
            category: draft.category,
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
            recipient,
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

    /// 清掉结束超过保留期的卡片。
    fn prune(&self, table: &mut Table, now: SystemTime) {
        let before = table.slots.len();
        let open: Vec<String> = table.windows.values().cloned().collect();
        table.slots.retain(|id, slot| {
            slot.waiter.is_some()
                || open.contains(id)
                || slot.ended.is_none_or(|ended| {
                    Duration::from_millis(now_ms(now).saturating_sub(ended)) < self.keep
                })
        });
        if table.slots.len() != before {
            self.bump();
        }
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
        let recipient = self.recipient(&card.id).flatten();
        let posted = self.notifier.post(recipient.as_deref(), card);
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
            if slot.recipient.as_deref() != recipient {
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

    /// 卡片变了：版本加一，挂着的长轮询知道。状态变化只改确认表，不再改任何 Matrix 消息。
    pub(super) fn bump(&self) {
        self.revision.send_modify(|revision| *revision += 1);
    }

    /// 卡片 `id` 发给谁（`Some(None)` 是主人）；没有这张卡片时是 `None`。
    pub fn recipient(&self, id: &str) -> Option<Option<String>> {
        self.lock().slots.get(id).map(|slot| slot.recipient.clone())
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
            &Languages::from_header(None),
        )
    }

    /// 带密钥的回答（ADR-0064）。
    pub fn answer_with_secrets(
        &self,
        id: &str,
        answer: Answer,
        choice_id: Option<&str>,
        secrets: SecretValues,
        settings: wonton_protocol::SettingValues,
        langs: &Languages,
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
        };
        if waiter.send(chosen).is_err() {
            slot.card.state = ConfirmationState::Expired;
            slot.ended = Some(now);
            self.activity_changed();
            self.bump();
            return Err(AnswerError::Expired);
        }
        slot.card.state = state.clone();
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
