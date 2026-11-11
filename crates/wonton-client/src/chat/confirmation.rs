//! 确认卡片（ADR-0016）：从时间线里认出核心发的卡片，算出现在该怎么显示，把主人的回答发回去。
//!
//! 卡片的状态以核心编辑后的消息为准。这台设备还知道两件核心的编辑可能没来得及告诉它的事：期限到了，以及自己回答的
//! 结果。核心重启后等着的确认没了，卡片不会再被编辑，所以期限由这里来算。手机的时钟快过 10 分钟时，还能回答的卡片会
//! 显示成过期；这时宁可让这次操作不执行，Bot 会收到“主人没有确认”，可以再发起一次。

use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::CONTENT_TYPE;
use serde_json::{Map, Value};
use tokio::sync::watch;
use url::Url;
use wonton_l10n::l10n;
use wonton_matrix::{EntryKind, Message, OwnedUserId, TimelineEntry, UserId, Vector};
use wonton_protocol::{
    Answer, CARD_FIELD, CORE_LOCALPART, ChatFailure, ChatProblem, ConfirmationAnswer,
    ConfirmationCard, ConfirmationReply, ConfirmationState, ConfirmationView, ConfirmationWindow,
    PendingConfirmation, ProxySettings, PushConfirmation, WindowEnd, WindowView, answer_path,
    window_path,
};

use super::login::{ErrorBody, proxy_url};
use crate::l10n::{CATALOG, language};

/// 回答的整个请求最多等这么久。核心收到回答马上返回，不等执行。
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

/// 这台设备回答过的卡片，按卡片 ID。随 App 的进程结束：重启后核心已经把卡片编辑好了，或者期限过了。
#[derive(Clone, Default)]
pub(crate) struct Answers {
    answers: HashMap<String, AnswerProgress>,
    stops: HashMap<String, StopProgress>,
}

#[derive(Clone)]
pub(super) enum AnswerProgress {
    Sending(Answer, Option<String>),
    /// 结果来自核心的回复，卡片的编辑可能还在路上。
    Settled(ConfirmationState),
    /// 核心不收这个回答，但不知道卡片现在是什么状态。
    Refused(ChatProblem),
    /// 没送到或者核心出错：可以再点。
    Failed(ChatProblem),
}

/// 主人点了卡片上放行时段的“停止”之后。
#[derive(Clone)]
pub(super) enum StopProgress {
    Sending,
    Stopped,
    /// 核心说没有生效中的时段：已经结束了，比如到期或者核心重启过。
    AlreadyEnded,
    /// 没送到或者核心出错：可以再点。
    Failed(ChatProblem),
}

impl Answers {
    /// 开始回答 `id`。这张卡片在发或者答完了时返回假。
    pub(super) fn start(&mut self, id: &str, answer: Answer, choice_id: Option<String>) -> bool {
        if matches!(self.answers.get(id), None | Some(AnswerProgress::Failed(_))) {
            self.answers
                .insert(id.to_owned(), AnswerProgress::Sending(answer, choice_id));
            return true;
        }
        false
    }

    pub(super) fn finish(&mut self, id: String, outcome: AnswerProgress) {
        self.answers.insert(id, outcome);
    }

    /// 开始停止 `id` 打开的时段。在发或者已经停了时返回假。
    pub(super) fn start_stop(&mut self, id: &str) -> bool {
        if matches!(self.stops.get(id), None | Some(StopProgress::Failed(_))) {
            self.stops.insert(id.to_owned(), StopProgress::Sending);
            return true;
        }
        false
    }

    pub(super) fn finish_stop(&mut self, id: String, outcome: StopProgress) {
        self.stops.insert(id, outcome);
    }
}

/// 核心账号：和登录的账号 `own_user` 在同一台服务器上。
pub(crate) fn core_account(own_user: &UserId) -> OwnedUserId {
    format!("@{CORE_LOCALPART}:{}", own_user.server_name())
        .parse()
        .expect("the account's server name with the fixed core localpart is still a valid user ID")
}

/// 消息是不是一张卡片：核心账号发的，内容读得懂。别人发的同样结构的内容是普通消息。比这个 App 新的核心写的、读不懂的卡片也当
/// 普通消息：`body` 是写给其他客户端看的纯文本，主人看得到，只是这里答不了。
fn parse_card(
    core_account: &UserId,
    sender: &UserId,
    custom: &Map<String, Value>,
) -> Option<ConfirmationCard> {
    if sender != core_account {
        return None;
    }
    let field = custom.get(CARD_FIELD)?;
    match serde_json::from_value(field.clone()) {
        Ok(card) => Some(card),
        Err(error) => {
            tracing::warn!("could not parse the confirmation card sent by the core: {error}");
            None
        }
    }
}

/// 收到推送的这条消息对应的卡片。卡片的编辑不推送，所以这里只看得到它刚发出时的样子。
pub(crate) enum PushedCard {
    /// 不是卡片，或者卡片已经有了结果（核心换了房间重发的那种）。
    None,
    Waiting(PushConfirmation),
    /// 推送来晚了，卡片已经过了期限：不用再提醒。
    Expired,
}

pub(crate) fn pushed_card(
    own_user: &UserId,
    sender: &UserId,
    custom: &Map<String, Value>,
) -> PushedCard {
    match parse_card(&core_account(own_user), sender, custom) {
        Some(card) if card.state == ConfirmationState::Pending => {
            if now_ms() >= card.expires_at_ms {
                PushedCard::Expired
            } else {
                PushedCard::Waiting(PushConfirmation {
                    id: card.id,
                    expires_at_ms: card.expires_at_ms,
                    origin: None,
                })
            }
        }
        _ => PushedCard::None,
    }
}

/// 一个房间里的卡片现在的情况。正在回答的两边都不算：回答成了还是没成，要等结果。
#[derive(Default)]
pub(super) struct RoomCards {
    /// 等主人回答的，和它们的期限。
    pub(super) pending: Vec<(u64, PendingConfirmation)>,
    /// 不用再回答的。
    pub(super) settled: Vec<String>,
}

/// 一条时间线订阅看卡片要用的东西。
#[derive(Clone)]
pub(crate) struct Cards {
    core_account: OwnedUserId,
    answers: watch::Receiver<Answers>,
}

impl Cards {
    /// `own_user`：登录的账号。核心账号和它在同一台服务器上。
    pub(super) fn new(own_user: &UserId, answers: watch::Receiver<Answers>) -> Self {
        Self {
            core_account: core_account(own_user),
            answers,
        }
    }

    /// 现在的回答进展和时间。
    pub(super) fn view(&self) -> CardsView {
        CardsView {
            core_account: self.core_account.clone(),
            answers: self.answers.borrow().clone(),
            now_ms: now_ms(),
        }
    }

    pub(super) async fn changed(&mut self) -> Result<(), watch::error::RecvError> {
        self.answers.changed().await
    }
}

/// 某一刻的回答进展和时间，同一份输入算出同样的显示。
pub(crate) struct CardsView {
    core_account: OwnedUserId,
    answers: Answers,
    now_ms: u64,
}

impl CardsView {
    pub(super) fn core_account(&self) -> &UserId {
        &self.core_account
    }

    fn card(&self, message: &Message) -> Option<ConfirmationCard> {
        parse_card(&self.core_account, &message.sender, &message.custom)
    }

    pub(super) fn is_card(&self, entry: &TimelineEntry) -> bool {
        matches!(&entry.kind, EntryKind::Message(message) if self.card(message).is_some())
    }

    /// `message` 是卡片时，它现在该怎么显示。
    pub(super) fn view_of(&self, message: &Message) -> Option<ConfirmationView> {
        let mut card = self.card(message)?;
        let mut sending = None;
        let mut sending_choice_id = None;
        let mut problem = None;
        if card.state == ConfirmationState::Pending {
            if self.now_ms >= card.expires_at_ms {
                card.state = ConfirmationState::Expired;
            }
            match self.answers.answers.get(&card.id) {
                Some(AnswerProgress::Sending(answer, choice_id)) => {
                    sending = Some(*answer);
                    sending_choice_id.clone_from(choice_id);
                }
                Some(AnswerProgress::Settled(state)) => card.state = state.clone(),
                Some(AnswerProgress::Refused(refused)) => {
                    card.choices.clear();
                    problem = Some(refused.clone());
                }
                Some(AnswerProgress::Failed(failed)) => problem = Some(failed.clone()),
                None => {}
            }
        }
        if card.state == ConfirmationState::Pending {
            card.choices
                .retain(|choice| choice.answer != Answer::Unknown);
        } else {
            card.choices.clear();
            problem = None;
            sending = None;
            sending_choice_id = None;
        }
        let window = card
            .window
            .as_ref()
            .map(|window| self.window_view(&card.id, window));
        Some(ConfirmationView {
            card,
            sending,
            sending_choice_id,
            problem,
            window,
        })
    }

    /// 时段以核心编辑后的卡片为准。核心没来得及编辑（比如停着）时，这里知道到期了，也知道自己停过。
    fn window_view(&self, id: &str, window: &ConfirmationWindow) -> WindowView {
        let stop = self.answers.stops.get(id);
        let mut window = window.clone();
        if window.ended.is_none() {
            window.ended = match stop {
                Some(StopProgress::Stopped) => Some(WindowEnd::Stopped),
                Some(StopProgress::AlreadyEnded) => Some(WindowEnd::Unknown),
                _ => (self.now_ms >= window.expires_at_ms).then_some(WindowEnd::Expired),
            };
        }
        match window.ended.map(|end| ended_note(end, window.allowed)) {
            Some(note) => WindowView::Ended { note },
            None => WindowView::Active {
                expires_at_ms: window.expires_at_ms,
                stopping: matches!(stop, Some(StopProgress::Sending)),
                problem: match stop {
                    Some(StopProgress::Failed(problem)) => Some(problem.clone()),
                    _ => None,
                },
            },
        }
    }

    /// 这台设备在本会话里回答过这张卡片（答了什么、收没收下都算）。回答只记到进程结束，
    /// 所以回答过它就不是启动时翻回来的历史结果：哪怕跟随第一次见到的就是结果，也认作本会话的。
    pub(super) fn answered_here(&self, card_id: &str) -> bool {
        self.answers.answers.contains_key(card_id)
    }

    /// `room_id` 的时间线里，卡片现在的情况。
    pub(super) fn standing(&self, room_id: &str, entries: &Vector<TimelineEntry>) -> RoomCards {
        let mut cards = RoomCards::default();
        for entry in entries {
            let EntryKind::Message(message) = &entry.kind else {
                continue;
            };
            let Some(view) = self.view_of(message) else {
                continue;
            };
            let card = view.card;
            if view.sending.is_some() {
                continue;
            }
            if card.state != ConfirmationState::Pending {
                cards.settled.push(card.id);
            } else if !card.choices.is_empty() {
                cards.pending.push((
                    card.expires_at_ms,
                    PendingConfirmation {
                        id: card.id,
                        room_id: room_id.to_owned(),
                        title: card.title,
                        requester: card.requester,
                        expires_at_ms: card.expires_at_ms,
                    },
                ));
            } else if view.problem.is_some() {
                // 核心不收回答了，比如主人在另一台设备上答过。没有问题却也没有按钮的，是这个 App 认不得它的回答，
                // 卡片还等着，只是这里答不了，不催主人。
                cards.settled.push(card.id);
            }
        }
        // SDK 打开时间线时先订阅更新再读缓存，这之间到的事件会进来两次。一张卡片只算一次。
        cards.settled.sort();
        cards.settled.dedup();
        let mut seen = HashSet::new();
        cards.pending.retain(|(_, card)| {
            cards.settled.binary_search(&card.id).is_err() && seen.insert(card.id.clone())
        });
        cards
    }

    /// 时间线里带插件 ID 的卡片，按 `(卡片 ID, 是否已经成功, 插件 ID)` 报。已经成功的包括历史里
    /// 本来就是结果的：补取只认本会话里亲眼看到从没结束变成成功的那次，历史结果由 follow_room 认出。
    pub(super) fn plugin_cards(
        &self,
        entries: &Vector<TimelineEntry>,
    ) -> Vec<(String, bool, String)> {
        entries
            .iter()
            .filter_map(|entry| {
                let EntryKind::Message(message) = &entry.kind else {
                    return None;
                };
                let view = self.view_of(message)?;
                let plugin_id = view.card.plugin_id.clone()?;
                let succeeded = matches!(view.card.state, ConfirmationState::Succeeded { .. });
                Some((view.card.id, succeeded, plugin_id))
            })
            .collect()
    }

    /// 还有多久有一张显示着等回答的卡片或者生效中的时段到期，到时候显示要变。已经到期的是零，尽快刷新。
    pub(super) fn next_expiry(&self, entries: &Vector<TimelineEntry>) -> Option<Duration> {
        let now = now_ms();
        entries
            .iter()
            .filter_map(|entry| match &entry.kind {
                EntryKind::Message(message) => self.card(message),
                _ => None,
            })
            .flat_map(|card| {
                let pending =
                    (card.state == ConfirmationState::Pending).then_some(card.expires_at_ms);
                let window = card
                    .window
                    .filter(|window| window.ended.is_none())
                    .map(|window| window.expires_at_ms);
                pending.into_iter().chain(window)
            })
            .filter(|expires_at_ms| *expires_at_ms > self.now_ms)
            .map(|expires_at_ms| expires_at_ms.saturating_sub(now))
            .min()
            .map(Duration::from_millis)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

/// 走本地代理的客户端，核心凭隧道身份认出这台已配对设备。
fn tunnel_client(proxy: &ProxySettings) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy_url(proxy))?)
        .timeout(ANSWER_TIMEOUT)
        .build()
}

/// `POST <核心地址>/_wonton/confirmations/{id}`：请求走本地代理，核心凭隧道身份认出这台已配对设备。
pub(super) async fn send_answer(
    core_url: &Url,
    proxy: &ProxySettings,
    id: &str,
    answer: ConfirmationAnswer,
) -> AnswerProgress {
    let failed = |reason, message: String| {
        AnswerProgress::Failed(ChatProblem {
            reason,
            message: l10n!(
                CATALOG,
                language(),
                "chat-confirmation-answer-failed",
                message = message
            ),
        })
    };
    let url = match core_url.join(&answer_path(id)) {
        Ok(url) => url,
        Err(error) => return failed(ChatFailure::InvalidInput, error.to_string()),
    };
    let client = match tunnel_client(proxy) {
        Ok(client) => client,
        Err(error) => return failed(ChatFailure::Upstream, error.to_string()),
    };
    let body = serde_json::to_vec(&answer).expect("the answer always serializes to JSON");
    let response = client
        .post(url)
        .header(CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await;
    // 请求可能已经到了核心，只是回应没回来。再点一次是安全的：核心只收一次，已经收下的会回“回答过了”。
    let lost = |error: reqwest::Error| {
        failed(
            ChatFailure::TransportUnavailable,
            l10n!(
                CATALOG,
                language(),
                "chat-confirmation-answer-lost",
                error = error.to_string()
            ),
        )
    };
    let response = match response {
        Ok(response) => response,
        Err(error) => return lost(error),
    };
    let status = response.status().as_u16();
    match response.bytes().await {
        Ok(body) => outcome(status, &body),
        Err(error) => lost(error),
    }
}

/// `DELETE <核心地址>/_wonton/confirmations/{id}/window`。
pub(super) async fn send_stop(core_url: &Url, proxy: &ProxySettings, id: &str) -> StopProgress {
    let failed = |reason, message: String| {
        StopProgress::Failed(ChatProblem {
            reason,
            message: l10n!(
                CATALOG,
                language(),
                "chat-confirmation-stop-failed",
                message = message
            ),
        })
    };
    let url = match core_url.join(&window_path(id)) {
        Ok(url) => url,
        Err(error) => return failed(ChatFailure::InvalidInput, error.to_string()),
    };
    let client = match tunnel_client(proxy) {
        Ok(client) => client,
        Err(error) => return failed(ChatFailure::Upstream, error.to_string()),
    };
    // 请求可能已经到了核心，只是回应没回来。再点一次是安全的：已经停了的会回 404。
    let response = match client.delete(url).send().await {
        Ok(response) => response,
        Err(error) => {
            return failed(
                ChatFailure::TransportUnavailable,
                l10n!(
                    CATALOG,
                    language(),
                    "chat-confirmation-stop-lost",
                    error = error.to_string()
                ),
            );
        }
    };
    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        return StopProgress::Stopped;
    }
    if status == 404 {
        return StopProgress::AlreadyEnded;
    }
    let body = response.bytes().await.unwrap_or_default();
    failed(failure_reason(status), error_message(status, &body))
}

/// 核心的错误响应里的原话。
fn error_message(status: u16, body: &[u8]) -> String {
    serde_json::from_slice::<ErrorBody>(body).map_or_else(
        |_| {
            l10n!(
                CATALOG,
                language(),
                "chat-confirmation-core-status",
                status = status
            )
        },
        |body| body.error.message,
    )
}

fn failure_reason(status: u16) -> ChatFailure {
    match status {
        401 | 403 => ChatFailure::Unauthorized,
        400 => ChatFailure::InvalidInput,
        // 本地代理连不上隧道时回 502，并在 x-wonton-tunnel-error 里写明原因。
        502 => ChatFailure::TransportUnavailable,
        503 => ChatFailure::NotReady,
        _ => ChatFailure::Upstream,
    }
}

/// 按核心的回应区分结果。
fn outcome(status: u16, body: &[u8]) -> AnswerProgress {
    if (200..300).contains(&status) {
        return match serde_json::from_slice::<ConfirmationReply>(body) {
            Ok(reply) => AnswerProgress::Settled(reply.state),
            Err(error) => AnswerProgress::Failed(ChatProblem {
                reason: ChatFailure::Upstream,
                message: l10n!(
                    CATALOG,
                    language(),
                    "chat-confirmation-answer-failed",
                    message = l10n!(
                        CATALOG,
                        language(),
                        "chat-confirmation-response-unreadable",
                        error = error.to_string()
                    )
                ),
            }),
        };
    }
    let detail = serde_json::from_slice::<ErrorBody>(body)
        .ok()
        .map(|body| body.error);
    let kind = detail.as_ref().map(|detail| detail.kind.as_str());
    let message = error_message(status, body);
    match (status, kind) {
        // 核心说过期了：和卡片被编辑成过期是一回事。
        (409, Some("expired")) => AnswerProgress::Settled(ConfirmationState::Expired),
        // 回答过了（比如主人的另一台设备），或者核心已经没有这个确认（比如重启过）：再点也没用。
        (404 | 409, _) => AnswerProgress::Refused(ChatProblem {
            reason: ChatFailure::InvalidInput,
            message,
        }),
        _ => AnswerProgress::Failed(ChatProblem {
            reason: failure_reason(status),
            message: l10n!(
                CATALOG,
                language(),
                "chat-confirmation-answer-failed",
                message = message
            ),
        }),
    }
}

/// 放行时段结束后的那一行，和核心编辑卡片时写的是同一条消息（`chat-window-ended`）。
fn ended_note(end: WindowEnd, allowed: Option<u32>) -> String {
    let reason = match end {
        WindowEnd::Expired => "expired",
        WindowEnd::Stopped => "stopped",
        WindowEnd::CoreRestarted => "restarted",
        WindowEnd::Unknown => "other",
    };
    l10n!(
        CATALOG,
        language(),
        "chat-window-ended",
        reason = reason,
        count = allowed.unwrap_or(0)
    )
}
