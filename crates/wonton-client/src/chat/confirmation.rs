//! 确认卡片（ADR-0016）：核心记着卡片，经“等你确认”列表（`GET /_wonton/api/confirmations`）和推送交给 App；
//! 这里合成每张卡片现在该怎么显示，把主人的回答和“停止放行时段”发回去。
//!
//! 卡片的正文以核心的列表和单张接口为准，时间线里不再有卡片。这台设备还知道两件核心可能没来得及告诉它的事：
//! 期限到了，以及自己回答的结果、自己停过的放行时段。核心重启后等着的确认没了，卡片不会再有更新，所以期限由
//! 这里来算。手机的时钟快过 10 分钟时，还能回答的卡片会显示成过期；这时宁可让这次操作不执行，Bot 会收到
//! “主人没有确认”，可以再发起一次。

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::CONTENT_TYPE;
use tokio::sync::watch;
use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{
    Answer, CONFIRMATIONS_PATH, CONFIRMATIONS_WAIT_SECS, ChatAction, ChatFailure, ChatProblem,
    ConfirmationAnswer, ConfirmationCard, ConfirmationList, ConfirmationReply, ConfirmationState,
    ConfirmationView, ConfirmationWindow, CoreError, ProxySettings, WindowEnd, WindowView,
    answer_path, confirmation_path, window_path,
};

use super::login::{ErrorBody, proxy_url};
use super::{ChatLink, invalid};
use crate::l10n::{CATALOG, language};

/// 回答的整个请求最多等这么久。核心收到回答马上返回，不等执行。
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

/// 长轮询最多挂 [`CONFIRMATIONS_WAIT_SECS`]，客户端的超时要比它长。
const POLL_TIMEOUT: Duration = Duration::from_secs(CONFIRMATIONS_WAIT_SECS + 10);

/// 这台设备回答过的卡片，按卡片 ID。随 App 的进程结束：重启后核心已经把卡片移出等回答的列表，或者期限过了。
#[derive(Clone, Default)]
pub(crate) struct Answers {
    answers: HashMap<String, AnswerProgress>,
    stops: HashMap<String, StopProgress>,
}

#[derive(Clone)]
pub(super) enum AnswerProgress {
    Sending(Answer, Option<String>),
    /// 结果来自核心的回复，列表可能还没更新。
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

    /// 这台设备在本进程里回答过这张卡片（答了什么、收没收下都算）。
    pub(super) fn answered_here(&self, card_id: &str) -> bool {
        self.answers.contains_key(card_id)
    }
}

/// 合成一张卡片现在该怎么显示：期限、这台设备的回答进展、放行时段。同一份输入算出同样的显示。
pub(crate) fn card_view(
    mut card: ConfirmationCard,
    answers: &Answers,
    now_ms: u64,
) -> ConfirmationView {
    let mut sending = None;
    let mut sending_choice_id = None;
    let mut problem = None;
    if card.state == ConfirmationState::Pending {
        if now_ms >= card.expires_at_ms {
            card.state = ConfirmationState::Expired;
        }
        match answers.answers.get(&card.id) {
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
        .map(|window| window_view(&card.id, window, answers, now_ms));
    ConfirmationView {
        card,
        sending,
        sending_choice_id,
        problem,
        window,
    }
}

/// 时段以核心的列表为准。核心没来得及更新（比如停着）时，这里知道到期了，也知道自己停过。
fn window_view(
    id: &str,
    window: &ConfirmationWindow,
    answers: &Answers,
    now_ms: u64,
) -> WindowView {
    let stop = answers.stops.get(id);
    let mut window = window.clone();
    if window.ended.is_none() {
        window.ended = match stop {
            Some(StopProgress::Stopped) => Some(WindowEnd::Stopped),
            Some(StopProgress::AlreadyEnded) => Some(WindowEnd::Unknown),
            _ => (now_ms >= window.expires_at_ms).then_some(WindowEnd::Expired),
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

/// 还有多久这张卡片要变显示：等回答的到期，或者生效中的时段到期。已经到期的是零，尽快刷新。
pub(super) fn next_wakeup_of_card(card: &ConfirmationCard, now_ms: u64) -> Option<Duration> {
    let pending = (card.state == ConfirmationState::Pending).then_some(card.expires_at_ms);
    let window = card
        .window
        .as_ref()
        .filter(|window| window.ended.is_none())
        .map(|window| window.expires_at_ms);
    pending
        .into_iter()
        .chain(window)
        .filter(|expires_at_ms| *expires_at_ms > now_ms)
        .map(|expires_at_ms| expires_at_ms.saturating_sub(now_ms))
        .min()
        .map(Duration::from_millis)
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

/// 走本地代理的客户端，核心凭隧道身份认出这台已配对设备。
fn tunnel_client(
    proxy: &ProxySettings,
    timeout: Duration,
) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy_url(proxy))?)
        .timeout(timeout)
        .build()
}

/// `GET <核心地址>/_wonton/api/confirmations?after=<revision>`：等回答的和结束的卡片。带 `after` 时核心没有更新的
/// 就挂住，所以超时比 [`CONFIRMATIONS_WAIT_SECS`] 长。
pub(crate) async fn fetch_list(
    core_url: Url,
    proxy: ProxySettings,
    after: Option<u64>,
) -> Result<ConfirmationList, ChatProblem> {
    let failed = |reason, message: String| ChatProblem {
        reason,
        message: l10n!(
            CATALOG,
            language(),
            "chat-confirmations-list-failed",
            message = message
        ),
    };
    let mut url = match core_url.join(CONFIRMATIONS_PATH) {
        Ok(url) => url,
        Err(error) => return Err(failed(ChatFailure::InvalidInput, error.to_string())),
    };
    if let Some(after) = after {
        url.query_pairs_mut()
            .append_pair("after", &after.to_string());
    }
    let client = match tunnel_client(&proxy, POLL_TIMEOUT) {
        Ok(client) => client,
        Err(error) => return Err(failed(ChatFailure::Upstream, error.to_string())),
    };
    let response = match client.get(url).send().await {
        Ok(response) => response,
        Err(error) => return Err(failed(ChatFailure::TransportUnavailable, error.to_string())),
    };
    let status = response.status().as_u16();
    let body = match response.bytes().await {
        Ok(body) => body,
        Err(error) => return Err(failed(ChatFailure::TransportUnavailable, error.to_string())),
    };
    if !(200..300).contains(&status) {
        return Err(failed(failure_reason(status), error_message(status, &body)));
    }
    serde_json::from_slice(&body).map_err(|error| {
        failed(
            ChatFailure::Upstream,
            l10n!(
                CATALOG,
                language(),
                "chat-confirmation-response-unreadable",
                error = error.to_string()
            ),
        )
    })
}

/// `GET <核心地址>/_wonton/api/confirmations/{id}`：一张卡片。`Ok(None)` 是核心说没有这张卡片：回答过了、
/// 结束太久被移出、或者核心重启过把它忘了。
pub(crate) async fn fetch_card(
    core_url: Url,
    proxy: ProxySettings,
    id: &str,
) -> Result<Option<ConfirmationCard>, ChatProblem> {
    let failed = |reason, message: String| ChatProblem {
        reason,
        message: l10n!(
            CATALOG,
            language(),
            "chat-confirmation-fetch-failed",
            message = message
        ),
    };
    let url = match core_url.join(&confirmation_path(id)) {
        Ok(url) => url,
        Err(error) => return Err(failed(ChatFailure::InvalidInput, error.to_string())),
    };
    let client = match tunnel_client(&proxy, ANSWER_TIMEOUT) {
        Ok(client) => client,
        Err(error) => return Err(failed(ChatFailure::Upstream, error.to_string())),
    };
    let response = match client.get(url).send().await {
        Ok(response) => response,
        Err(error) => return Err(failed(ChatFailure::TransportUnavailable, error.to_string())),
    };
    let status = response.status().as_u16();
    if status == 404 {
        return Ok(None);
    }
    let body = match response.bytes().await {
        Ok(body) => body,
        Err(error) => return Err(failed(ChatFailure::TransportUnavailable, error.to_string())),
    };
    if !(200..300).contains(&status) {
        return Err(failed(failure_reason(status), error_message(status, &body)));
    }
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| failed(ChatFailure::Upstream, error.to_string()))
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
    let client = match tunnel_client(proxy, ANSWER_TIMEOUT) {
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
    let client = match tunnel_client(proxy, ANSWER_TIMEOUT) {
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
pub(super) fn error_message(status: u16, body: &[u8]) -> String {
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

pub(super) fn failure_reason(status: u16) -> ChatFailure {
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
        // 核心说过期了：和卡片被标成过期是一回事。
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

/// 放行时段结束后的那一行，和核心写的是同一句话（`chat-window-ended`）。
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

/// 占住的一张卡片和发这次回答要用的东西：发出去、按结果写回进展。
struct PreparedAnswer {
    answers: watch::Sender<Answers>,
    core_url: Url,
    proxy: ProxySettings,
    id: String,
    body: ConfirmationAnswer,
}

impl PreparedAnswer {
    /// 发出回答并写回进展；只有核心明确确认收下回答才算成功。
    async fn send(self) -> Result<(), CoreError> {
        let outcome = send_answer(&self.core_url, &self.proxy, &self.id, self.body).await;
        let refused = match &outcome {
            AnswerProgress::Settled(
                ConfirmationState::Running
                | ConfirmationState::Succeeded { .. }
                | ConfirmationState::Rejected,
            ) => None,
            AnswerProgress::Failed(problem) | AnswerProgress::Refused(problem) => {
                Some(problem.clone())
            }
            AnswerProgress::Settled(_) | AnswerProgress::Sending(..) => Some(ChatProblem {
                reason: ChatFailure::InvalidInput,
                message: l10n!(CATALOG, language(), "chat-unknown-answer"),
            }),
        };
        self.answers
            .send_modify(|answers| answers.finish(self.id, outcome));
        match refused {
            Some(problem) => Err(answer_error(&problem)),
            None => Ok(()),
        }
    }
}

/// 回答失败的类别（给通知上写原因）。
fn answer_error(problem: &ChatProblem) -> CoreError {
    let message = problem.message.clone();
    match problem.reason {
        ChatFailure::InvalidInput => CoreError::InvalidInput { reason: message },
        ChatFailure::NotReady => CoreError::NotReady { reason: message },
        ChatFailure::TransportUnavailable => CoreError::TransportUnavailable { reason: message },
        ChatFailure::Unauthorized | ChatFailure::Forbidden => {
            CoreError::Unauthorized { reason: message }
        }
        ChatFailure::Upstream | ChatFailure::Other => CoreError::Upstream { reason: message },
    }
}

impl ChatLink {
    /// 确认卡片的操作：回答、停掉放行时段、重试取列表。`dispatch` 里确认的动作才路由到这里。
    pub(super) fn confirmation_action(&self, action: ChatAction) -> Result<(), CoreError> {
        match action {
            ChatAction::AnswerConfirmation {
                id,
                answer,
                choice_id,
                secrets,
                settings,
                devices,
            } => self.answer_confirmation(id, answer, choice_id, secrets, settings, devices),
            ChatAction::StopConfirmationWindow { id } => self.stop_confirmation_window(id),
            ChatAction::RetryConfirmations => {
                self.retry_confirmations.notify_one();
                Ok(())
            }
            _ => unreachable!("dispatch only routes confirmation actions here"),
        }
    }

    /// 主人点了确认卡片上的按钮：经隧道发给核心，进展放进 `answers`。同一张卡片在发或者答完了就不再发：核心那边
    /// 也只收一次，这里是不让连点变成“回答过了”的报错。
    pub(super) fn answer_confirmation(
        &self,
        id: String,
        answer: Answer,
        choice_id: Option<String>,
        secrets: wonton_protocol::SecretValues,
        settings: wonton_protocol::SettingValues,
        devices: Vec<String>,
    ) -> Result<(), CoreError> {
        let answer = self.prepare_answer(id, answer, choice_id, secrets, settings, devices)?;
        let Some(answer) = answer else {
            return Ok(());
        };
        self.runtime.spawn(answer.send());
        Ok(())
    }

    /// 通知动作等冷启动连接和回答结果，总计最多 30 秒。错误同时交回通知和卡片视图。
    pub(crate) async fn answer_from_push(
        &self,
        id: String,
        answer: Answer,
        choice_id: Option<String>,
    ) -> Result<(), CoreError> {
        let sending = matches!(
            self.answers.borrow().answers.get(&id),
            Some(AnswerProgress::Sending(..))
        );
        if sending {
            return Err(CoreError::NotReady {
                reason: l10n!(CATALOG, language(), "chat-action-not-connected"),
            });
        }
        let mut owns_answer = false;
        let result = tokio::time::timeout(ANSWER_TIMEOUT, async {
            let (core_url, proxy) = self.push_home().await?;
            let card = fetch_card(core_url, proxy, &id)
                .await
                .map_err(|problem| answer_error(&problem))?
                .ok_or_else(|| invalid(&l10n!(CATALOG, language(), "chat-unknown-answer")))?;
            if card.state != ConfirmationState::Pending
                || !card.secret_inputs.is_empty()
                || card.settings.is_some()
                || !card
                    .choices
                    .iter()
                    .any(|choice| choice.answer == answer && choice.choice_id == choice_id)
            {
                return Err(invalid(&l10n!(CATALOG, language(), "chat-unknown-answer")));
            }
            match self.prepare_answer(
                id.clone(),
                answer,
                choice_id,
                Default::default(),
                Default::default(),
                Default::default(),
            )? {
                Some(answer) => {
                    owns_answer = true;
                    answer.send().await
                }
                None => Err(CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "chat-action-not-connected"),
                }),
            }
        })
        .await
        .unwrap_or_else(|_| {
            Err(CoreError::TransportUnavailable {
                reason: l10n!(CATALOG, language(), "chat-send-no-response"),
            })
        });
        if let Err(error) = &result {
            self.answers.send_modify(|answers| {
                // A second notification action must not overwrite the first request's progress.
                if !owns_answer
                    && matches!(answers.answers.get(&id), Some(AnswerProgress::Sending(..)))
                {
                    return;
                }
                answers.finish(
                    id,
                    AnswerProgress::Failed(ChatProblem {
                        reason: match error {
                            CoreError::InvalidInput { .. } => ChatFailure::InvalidInput,
                            CoreError::Unauthorized { .. } => ChatFailure::Unauthorized,
                            CoreError::TransportUnavailable { .. } => {
                                ChatFailure::TransportUnavailable
                            }
                            CoreError::NotReady { .. } => ChatFailure::NotReady,
                            _ => ChatFailure::Upstream,
                        },
                        message: error.to_string(),
                    }),
                );
            });
        }
        result
    }

    /// 外层通知动作持有总期限；这里等待隧道恢复，不把冷启动误判成永久未连接。
    pub(super) async fn push_home(&self) -> Result<(Url, ProxySettings), CoreError> {
        let mut home = self.home.subscribe();
        loop {
            let proxy = home.borrow_and_update().proxy.clone();
            if let Some(proxy) = proxy
                && let Some(url) = self.home.core_url()
            {
                return Ok((url, proxy));
            }
            if home.changed().await.is_err() {
                return Err(CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "chat-action-not-connected"),
                });
            }
        }
    }

    /// 校验并占住这张卡片，返回一个发出去、写回进展的 future。已经答过、正在发时返回空。
    fn prepare_answer(
        &self,
        id: String,
        answer: Answer,
        choice_id: Option<String>,
        secrets: wonton_protocol::SecretValues,
        settings: wonton_protocol::SettingValues,
        devices: Vec<String>,
    ) -> Result<Option<PreparedAnswer>, CoreError> {
        if answer == Answer::Unknown {
            return Err(invalid(&l10n!(CATALOG, language(), "chat-unknown-answer")));
        }
        let not_connected = || CoreError::NotReady {
            reason: l10n!(CATALOG, language(), "chat-action-not-connected"),
        };
        let proxy = self.home.view().proxy.ok_or_else(not_connected)?;
        let core_url = self.home.core_url().ok_or_else(not_connected)?;
        if !self
            .answers
            .send_if_modified(|answers| answers.start(&id, answer, choice_id.clone()))
        {
            return Ok(None);
        }
        Ok(Some(PreparedAnswer {
            answers: self.answers.clone(),
            core_url,
            proxy,
            id,
            body: ConfirmationAnswer {
                answer,
                choice_id,
                secrets,
                settings,
                devices,
            },
        }))
    }

    /// 主人点了卡片上放行时段的“停止”：经隧道发给核心，进展放进 `answers`。
    pub(super) fn stop_confirmation_window(&self, id: String) -> Result<(), CoreError> {
        let not_connected = || CoreError::NotReady {
            reason: l10n!(CATALOG, language(), "chat-action-not-connected"),
        };
        let proxy = self.home.view().proxy.ok_or_else(not_connected)?;
        let core_url = self.home.core_url().ok_or_else(not_connected)?;
        if !self
            .answers
            .send_if_modified(|answers| answers.start_stop(&id))
        {
            return Ok(());
        }
        let answers = self.answers.clone();
        self.runtime.spawn(async move {
            let outcome = send_stop(&core_url, &proxy, &id).await;
            answers.send_modify(|answers| answers.finish_stop(id, outcome));
        });
        Ok(())
    }
}
