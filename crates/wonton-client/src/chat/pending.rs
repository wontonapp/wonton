//! “等你确认”列表（ADR-0016）：长轮询核心的确认接口，把等回答的、结束的卡片交给壳。主人在别的房间、别的页面
//! 时，壳靠 `ChatViewModel::pending_confirmations` 提示有卡片要回答。
//!
//! 跟着家里的连接走，不和 Matrix 会话绑定：卡片不是 Matrix 消息，换登录、重连都不动这份状态；连不上核心时如实
//! 报问题、留上次的列表，退避重试，`Action::RetryConfirmations` 立即重试。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::sync::watch;
use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{
    ChatProblem, ChatViewModel, ConfirmationList, ConfirmationState, ConfirmationView,
    ConfirmationsViewModel, HomeViewModel, PendingConfirmation, ProxySettings,
};

use super::confirmation::{Answers, card_view, fetch_list, next_wakeup_of_card, now_ms};
use super::{CardSuccess, ChatLink};
use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};

/// 长轮询失败后的退避：连不上一两次多半是抖动，翻倍着退，到上限为止。
const BACKOFF_START: Duration = Duration::from_secs(2);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// 短确认只发普通通知；持续等主人一分钟的 Bot 任务才成为进行中通知（ADR-0101 §4）。
const LONG_WAIT_MS: u64 = 60_000;

/// 长轮询“等你确认”列表。三份产出都从核心的同一份列表加这台设备的回答进展算出来：
/// - 壳订阅的 [`ConfirmationsViewModel`]：等回答的、结束的、取没取到过、最近一次的问题；
/// - [`ChatViewModel::pending_confirmations`] 和 `settled_confirmations`：提示条和撤通知的依据；
/// - 打开的房间要显示的卡片：时间线的出口按 `origin.room_id` 过滤后放进 `TimelineUpdate::confirmations`。
pub(super) async fn follow_cards(chat: Weak<ChatLink>, home: watch::Receiver<HomeViewModel>) {
    let Some(link) = chat.upgrade() else { return };
    let mut follower = Follower {
        answers: link.answers.subscribe(),
        retry: link.retry_confirmations.clone(),
        chat_view: link.view.clone(),
        vm_out: link.confirmations.clone(),
        rooms_out: link.room_cards.clone(),
        succeeded: link.card_success.clone(),
        home_link: link.home.clone(),
        home,
        latest: None,
        after: None,
        problem: None,
        backoff: Duration::ZERO,
        transitions: Witnessed::default(),
    };
    drop(link);
    follower.publish().await;
    follower.run().await;
}

/// 跟随者的状态。列表是核心给的；其余是这台设备自己知道的补充。
struct Follower {
    answers: watch::Receiver<Answers>,
    retry: Arc<tokio::sync::Notify>,
    chat_view: watch::Sender<ChatViewModel>,
    vm_out: watch::Sender<ConfirmationsViewModel>,
    rooms_out: watch::Sender<Arc<Vec<ConfirmationView>>>,
    succeeded: CardSuccess,
    home_link: Arc<HomeLink>,
    home: watch::Receiver<HomeViewModel>,
    /// 核心的最近一份列表（取到过的）。
    latest: Option<ConfirmationList>,
    /// 下次长轮询带的 `after`。
    after: Option<u64>,
    problem: Option<ChatProblem>,
    /// 下一次取列表前要等多久（失败后翻倍）。
    backoff: Duration,
    transitions: Witnessed,
}

/// 这个进程里“看着结束”的过渡：亲眼见过在等的、确认过结束的、报过成功结果的。
#[derive(Default)]
struct Witnessed {
    /// 见过还在等回答的卡片：它结束时就算“看着结束的”，会话里还显示一会儿结果。
    seen_pending: HashSet<String>,
    /// 看着结束的（亲眼见过在等，或这台设备回答过）：会话里显示结果，成功的补取一次已装列表。
    finished: HashSet<String>,
    /// 报过成功结果的卡片：重复见到的同一张不再报。
    reported: HashSet<String>,
    /// 只记录亲眼看到的等待，不从到期时间反推任务何时开始；重启后重新计时。
    waiting_since: HashMap<String, u64>,
}

impl Follower {
    async fn run(mut self) {
        // 挂着的取列表请求单独跑：它一挂就是 `CONFIRMATIONS_WAIT_SECS`，回答的进展和卡片到期不能跟着等。
        let mut in_flight: Option<tokio::task::JoinHandle<Result<ConfirmationList, ChatProblem>>> =
            None;
        loop {
            if in_flight.is_none() {
                let Some((core_url, proxy)) = self.wait_ready().await else {
                    return;
                };
                in_flight = Some(tokio::spawn(fetch_list(core_url, proxy, self.after)));
            }
            let Some(mut task) = in_flight.take() else {
                unreachable!()
            };
            match self.wait_for_event(&mut task).await {
                Some(Ok(list)) => {
                    self.after = Some(list.revision);
                    self.latest = Some(list);
                    self.problem = None;
                    self.backoff = Duration::ZERO;
                }
                Some(Err(next)) => {
                    self.problem = Some(next);
                    self.backoff = if self.backoff.is_zero() {
                        BACKOFF_START
                    } else {
                        (self.backoff * 2).min(BACKOFF_MAX)
                    };
                }
                // 家里的连接或回答的通道没了：作用域在收尾，任务跟着停。
                None => return,
            }
            self.publish().await;
        }
    }

    /// 等在飞的取列表请求回来。等的过程中回答的进展、卡片到期、重试和连接变化都会就地重算显示；
    /// 家里的连接或回答的通道没了返回 `None`（作用域在收尾）。
    async fn wait_for_event(
        &mut self,
        task: &mut tokio::task::JoinHandle<Result<ConfirmationList, ChatProblem>>,
    ) -> Option<Result<ConfirmationList, ChatProblem>> {
        let mut wakeup = next_wakeup(&self.latest, &self.transitions);
        loop {
            tokio::select! {
                fetched = &mut *task => {
                    return Some(fetched.unwrap_or_else(|error| {
                        Err(transport(format!("the list request failed: {error}")))
                    }));
                }
                changed = self.answers.changed() => {
                    changed.ok()?;
                    self.publish().await;
                    wakeup = next_wakeup(&self.latest, &self.transitions);
                }
                _ = async {
                    match wakeup {
                        Some(delay) => tokio::time::sleep(delay).await,
                        None => std::future::pending().await,
                    }
                } => {
                    // 期限到了：重新算显示，卡片在列表里换成“已过期”；剩下的到期接着等。
                    self.publish().await;
                    wakeup = next_wakeup(&self.latest, &self.transitions);
                }
                _ = self.retry.notified() => {
                    // 正在挂着的请求等它自己回来；退避清零，下一次取列表立即发。
                    self.backoff = Duration::ZERO;
                }
                changed = self.home.changed() => {
                    changed.ok()?;
                    // 连接变了：挂着的请求多半要失败，走正常的失败路径。
                }
            }
        }
    }

    /// 连接没就绪就不打核心：留上次的列表，一次都没取到过就还显示加载中。失败过的先等退避；
    /// `Action::RetryConfirmations` 立即放行。连接没了（作用域释放）返回 `None`。
    async fn wait_ready(&mut self) -> Option<(Url, ProxySettings)> {
        if !self.backoff.is_zero() {
            tokio::select! {
                _ = tokio::time::sleep(self.backoff) => {}
                _ = self.retry.notified() => {}
                changed = self.home.changed() => {
                    changed.ok()?;
                }
            }
        }
        loop {
            let proxy = self.home.borrow_and_update().proxy.clone();
            if let (Some(proxy), Some(core_url)) = (proxy, self.home_link.core_url()) {
                return Some((core_url, proxy));
            }
            tokio::select! {
                changed = self.home.changed() => {
                    changed.ok()?;
                }
                _ = self.retry.notified() => {}
            }
        }
    }

    /// 把算好的显示发出去。卡片在这台设备上看着变成成功时，先就地补取已装列表（等测试钩子放行），再发列表：
    /// 壳看到卡片结束时，补不补取已经定了。
    async fn publish(&mut self) {
        let answers = self.answers.borrow().clone();
        let derived = derive(
            self.latest.as_ref(),
            &answers,
            now_ms(),
            &mut self.transitions,
        );
        let mut successes = Vec::new();
        for view in &derived.finished {
            if !matches!(view.card.state, ConfirmationState::Succeeded { .. }) {
                continue;
            }
            let Some(plugin_id) = &view.card.plugin_id else {
                continue;
            };
            if !self.transitions.finished.contains(&view.card.id) {
                continue;
            }
            if self.transitions.reported.insert(view.card.id.clone()) {
                successes.push(plugin_id.clone());
            }
        }
        for plugin_id in successes {
            #[cfg(not(feature = "test-hooks"))]
            (self.succeeded)(&plugin_id);
            #[cfg(feature = "test-hooks")]
            (self.succeeded)(&plugin_id).await;
        }
        self.vm_out.send_replace(ConfirmationsViewModel {
            pending: derived.pending.clone(),
            finished: derived.finished.clone(),
            // 联系人请求跟卡片坐同一班列表（ADR-0051 的接法在联系人那头），这里照核心给的顺序原样带过去。
            contact_requests: self
                .latest
                .as_ref()
                .map(|list| list.contact_requests.clone())
                .unwrap_or_default(),
            loading: self.latest.is_none(),
            problem: self.problem.clone(),
        });
        self.rooms_out.send_replace(Arc::new(derived.rooms.clone()));
        let chat_pending = derived.chat_pending.clone();
        let chat_settled = derived.chat_settled.clone();
        self.chat_view.send_if_modified(|view| {
            if view.pending_confirmations == chat_pending
                && view.settled_confirmations == chat_settled
            {
                return false;
            }
            view.pending_confirmations.clone_from(&chat_pending);
            view.settled_confirmations.clone_from(&chat_settled);
            true
        });
    }
}

/// 等回答的卡片和生效中的时段里，最近的到期还差多久；已经到期的显示刚发布时就算过了，不用再等。
fn next_wakeup(list: &Option<ConfirmationList>, transitions: &Witnessed) -> Option<Duration> {
    let now = now_ms();
    list.as_ref()?
        .confirmations
        .iter()
        .filter_map(|card| next_wakeup_of_card(card, now))
        .filter(|delay| !delay.is_zero())
        .chain(transitions.waiting_since.values().filter_map(|started| {
            let due = started.saturating_add(LONG_WAIT_MS);
            (due > now).then(|| Duration::from_millis(due - now))
        }))
        .min()
}

fn transport(message: String) -> ChatProblem {
    ChatProblem {
        reason: wonton_protocol::ChatFailure::TransportUnavailable,
        message: l10n!(
            CATALOG,
            language(),
            "chat-confirmations-list-failed",
            message = message
        ),
    }
}

/// 一份核心列表加这台设备的回答进展，算出各处要显示的东西。
struct Derived {
    /// 等回答的，最早到期的在前。正在发给核心的也在里面（`sending` 标着）。
    pending: Vec<ConfirmationView>,
    /// 结束的：显示成过期的（核心还没标，按到期晚的在前）在前，核心给的（保序）在后。
    finished: Vec<ConfirmationView>,
    /// [`ChatViewModel::pending_confirmations`]。
    chat_pending: Vec<PendingConfirmation>,
    /// [`ChatViewModel::settled_confirmations`]。
    chat_settled: Vec<String>,
    /// 打开的房间能显示的：等回答的，加上看着结束的。
    rooms: Vec<ConfirmationView>,
}

fn derive(
    list: Option<&ConfirmationList>,
    answers: &Answers,
    now: u64,
    transitions: &mut Witnessed,
) -> Derived {
    let Some(list) = list else {
        return Derived {
            pending: Vec::new(),
            finished: Vec::new(),
            chat_pending: Vec::new(),
            chat_settled: Vec::new(),
            rooms: Vec::new(),
        };
    };
    let mut pending = Vec::new();
    // 核心还挂在等回答的列表里、这台设备知道已经结束了的（到期了，自己答过了，或核心不收回答了）。
    let mut ended_early = Vec::new();
    for card in &list.confirmations {
        let view = card_view(card.clone(), answers, now);
        if view.sending.is_some() {
            // 正在发给核心：列表和会话里都显示发送中，等结果；提示条不再催（这台设备已经在答了）。
            transitions.seen_pending.insert(card.id.clone());
            pending.push(view);
        } else if view.card.state != ConfirmationState::Pending {
            ended_early.push(view);
        } else if view.card.choices.is_empty() && view.problem.is_some() {
            // 核心不收回答了（主人的另一台设备答过、没有这个确认）：对这台设备来说已经结束，如实带着原因。
            ended_early.push(view);
        } else {
            transitions.seen_pending.insert(card.id.clone());
            pending.push(view);
        }
    }
    ended_early.sort_by_key(|view| std::cmp::Reverse(view.card.expires_at_ms));
    let mut finished = Vec::new();
    let mut chat_settled = Vec::new();
    for view in ended_early {
        note_ended(view, answers, transitions, &mut chat_settled, &mut finished);
    }
    for card in &list.finished {
        note_ended(
            card_view(card.clone(), answers, now),
            answers,
            transitions,
            &mut chat_settled,
            &mut finished,
        );
    }
    pending.sort_by_key(|view| view.card.expires_at_ms);
    transitions
        .waiting_since
        .retain(|id, _| pending.iter().any(|view| view.card.id == *id));
    for view in &pending {
        if view.sending.is_none()
            && matches!(
                view.card.requester,
                Some(wonton_protocol::ConfirmationRequester::Bot { .. })
            )
        {
            transitions
                .waiting_since
                .entry(view.card.id.clone())
                .or_insert(now);
        }
    }
    // 会话横幅只催能在这里回答、还没在答的：正在发的和认不得它的回答的卡片（比这个 App 新的核心写的）不催主人。
    let chat_pending = pending
        .iter()
        .filter(|view| view.sending.is_none() && !view.card.choices.is_empty())
        .map(|view| PendingConfirmation {
            id: view.card.id.clone(),
            room_id: view
                .card
                .origin
                .as_ref()
                .map(|origin| origin.room_id.clone()),
            title: view.card.title.clone(),
            requester: view.card.requester.clone(),
            expires_at_ms: view.card.expires_at_ms,
            ongoing: transitions
                .waiting_since
                .get(&view.card.id)
                .is_some_and(|started| now.saturating_sub(*started) >= LONG_WAIT_MS),
        })
        .collect();
    let rooms = pending
        .iter()
        .chain(
            finished
                .iter()
                .filter(|view| transitions.finished.contains(&view.card.id)),
        )
        .cloned()
        .collect();
    Derived {
        pending,
        finished,
        chat_pending,
        chat_settled,
        rooms,
    }
}

/// 一张结束的卡片：记下“看着结束”的过渡（见过它在等，或这台设备回答过它），列进不用回答的。
fn note_ended(
    view: ConfirmationView,
    answers: &Answers,
    transitions: &mut Witnessed,
    chat_settled: &mut Vec<String>,
    finished: &mut Vec<ConfirmationView>,
) {
    let was_pending = transitions.seen_pending.remove(&view.card.id);
    if was_pending || answers.answered_here(&view.card.id) {
        transitions.finished.insert(view.card.id.clone());
    }
    chat_settled.push(view.card.id.clone());
    finished.push(view);
}

#[cfg(test)]
#[path = "pending_tests.rs"]
mod tests;
