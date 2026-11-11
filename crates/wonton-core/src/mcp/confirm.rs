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
use tokio::sync::oneshot;
use wonton_protocol::{
    Answer, CardChoice, ConfirmationCard, ConfirmationState, SecretInput, SecretValues,
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

/// 回答过、过期的确认留这么久，之后再回答得到“不存在”。
const KEEP_FINISHED: Duration = Duration::from_secs(24 * 3600);

/// 放行时段默认多长（`WONTON_CONFIRMATION_WINDOW_SECS`，ADR-0024）。
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(30 * 60);

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
    /// 同一个（Bot，插件）有了新的确认请求，旧的这份被取代（ADR-0033 的暂存），等新的那份。
    Superseded,
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
    /// 卡片要填的密钥不对：缺必填项、多出不认识的项，或者这张卡片不收密钥。卡片照常等回答。
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
    /// 这张卡片的发送方：默认是通知房间，Bot 钉页的同意是它自己的房间。
    notifier: Arc<dyn Notifier>,
    /// 这张卡片能打开放行时段时，时段管哪个插件。
    scope: Option<String>,
    /// 见 [`Ticket::window`]。
    released_by: Option<String>,
    card: ConfirmationCard,
    handle: Option<String>,
    waiter: Option<oneshot::Sender<Chosen>>,
    /// 这张卡片要填的密钥（ADR-0064）；不填密钥的卡片是空的。
    secret_inputs: Vec<SecretInput>,
    created: SystemTime,
}

#[derive(Default)]
struct Table {
    slots: HashMap<String, Slot>,
    /// 生效中的放行时段：（Bot，插件）→ 打开它的卡片。其余状态在卡片的 `window` 里。
    windows: HashMap<(PluginId, String), String>,
    /// 上次运行留下、卡片还没改成“已结束”的时段。
    leftovers: HashMap<String, windows::Persisted>,
}

pub struct Confirmations {
    notifier: Arc<dyn Notifier>,
    ttl: Duration,
    window: Duration,
    /// 记着哪些卡片显示着时段，核心重启后才改得了它们。判定不读它。
    windows_file: Option<PathBuf>,
    table: Mutex<Table>,
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
            windows_file: None,
            table: Mutex::new(Table::default()),
            activity: RwLock::new(None),
        }
    }

    /// 默认的卡片发送方（通知房间）。
    pub(super) fn default_notifier(&self) -> &dyn Notifier {
        &*self.notifier
    }

    /// 同 [`Self::default_notifier`]，给要留着发送方的调用。
    pub(super) fn default_notifier_arc(&self) -> Arc<dyn Notifier> {
        self.notifier.clone()
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Table> {
        self.table.lock().expect("确认表锁中毒")
    }

    /// 请主人确认，等到主人回答或者过期。确认了返回 [`Ticket`]，执行完要调 [`Self::finish`]。
    pub async fn ask(&self, caller: &PluginId, draft: Draft) -> Result<Ticket, Refusal> {
        self.ask_with(self.notifier.clone(), caller, draft).await
    }

    /// 同 [`Self::ask`]，卡片用别的发送方：Bot 往面板钉页的同意发在 Bot 自己的房间里。
    /// 之后的更新仍走默认的通知房间，它按卡片发出去的房间编辑。
    pub async fn ask_with(
        &self,
        notifier: Arc<dyn Notifier>,
        caller: &PluginId,
        draft: Draft,
    ) -> Result<Ticket, Refusal> {
        let (card, answered) = self.open(notifier.clone(), caller, draft)?;
        let chosen = self.deliver(&*notifier, &card, answered).await?;
        self.publish(&card.id).await;
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
            // 不认识的回答到不了这里（`answer` 挡掉了）；真到了也不算同意。
            // 拒绝时 `deliver` 已经发布过卡片状态。
            Answer::Reject | Answer::Unknown => Err(Refusal::Rejected),
        }
    }

    /// 登记一张等回答的卡片，还没发出去。
    pub(super) fn open(
        &self,
        notifier: Arc<dyn Notifier>,
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
            plugin_id: draft.plugin_id,
            secret_inputs: draft.secret_inputs.clone(),
            settings: draft.settings.clone(),
            window: None,
            requester: None,
            origin: None,
            audience: wonton_protocol::CardAudience::Keeper,
            answered_by: None,
            category: None,
        };
        let (waiter, answered) = oneshot::channel();
        let mut table = self.lock();
        let open: Vec<String> = table.windows.values().cloned().collect();
        table.slots.retain(|id, slot| {
            slot.waiter.is_some()
                || open.contains(id)
                || created.duration_since(slot.created).unwrap_or_default() < KEEP_FINISHED
        });
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
            notifier,
            scope: draft.scope,
            released_by: None,
            card: card.clone(),
            handle: None,
            waiter: Some(waiter),
            secret_inputs: draft.secret_inputs,
            created,
        };
        table.slots.insert(card.id.clone(), slot);
        self.activity_changed();
        Ok((card, answered))
    }

    /// 把 [`Self::open`] 登记的卡片发给主人，等回答。回答之后卡片的变化由调用方发布。
    pub(super) async fn deliver(
        &self,
        notifier: &dyn Notifier,
        card: &ConfirmationCard,
        answered: oneshot::Receiver<Chosen>,
    ) -> Result<Chosen, Refusal> {
        let expires =
            Duration::from_millis(card.expires_at_ms.saturating_sub(now_ms(SystemTime::now())));
        let deadline = tokio::time::Instant::now() + expires;
        let handle = match tokio::time::timeout_at(deadline, notifier.post(card)).await {
            Ok(Ok(handle)) => handle,
            Ok(Err(e)) => {
                self.lock().slots.remove(&card.id);
                self.activity_changed();
                return Err(Refusal::Unavailable(e));
            }
            Err(_) => {
                self.settle(&card.id, ConfirmationState::Expired).await;
                return Err(Refusal::Expired);
            }
        };
        if let Some(slot) = self.lock().slots.get_mut(&card.id) {
            slot.handle = Some(handle);
        }
        match tokio::time::timeout_at(deadline, answered).await {
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
            .slots
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
            let missing: Vec<&str> = slot
                .secret_inputs
                .iter()
                .filter(|i| i.required && !secrets.contains_key(&i.name))
                .map(|i| i.name.as_str())
                .collect();
            if !missing.is_empty() {
                return Err(AnswerError::Secrets(crate::l10n::text(
                    langs,
                    "plugin-secret-required-missing",
                    &[("names", crate::l10n::list(langs, &missing).into())],
                )));
            }
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
            self.activity_changed();
            return Err(AnswerError::Expired);
        }
        slot.card.state = state.clone();
        if answer == Answer::AllowWindow {
            self.open_window(&mut table, id, now);
        }
        self.activity_changed();
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
        self.publish(id).await;
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
                self.activity_changed();
            }
            slot.card.state = state;
        }
        self.publish(id).await;
    }

    /// 把卡片现在的状态发给主人。更新失败只记日志：执行已经发生了，工具的结果照样返回给 Agent。
    pub(super) async fn publish(&self, id: &str) {
        let current = self
            .lock()
            .slots
            .get(id)
            .and_then(|slot| Some((slot.handle.clone()?, slot.card.clone())));
        let Some((handle, card)) = current else {
            return;
        };
        let notifier = self
            .lock()
            .slots
            .get(id)
            .map(|slot| slot.notifier.clone())
            .unwrap_or_else(|| self.notifier.clone());
        match notifier.update(&handle, &card).await {
            Ok(None) => {}
            Ok(Some(moved)) => {
                tracing::warn!(confirmation = %id, "原房间不能用了，确认卡片重发到了当前的房间");
                if let Some(slot) = self.lock().slots.get_mut(id) {
                    slot.handle = Some(moved);
                }
            }
            Err(e) => tracing::error!(confirmation = %id, error = %e, "更新确认卡片失败"),
        }
    }
}
