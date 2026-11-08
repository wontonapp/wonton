//! 收消息、跑一轮、收尾：分流到会话、拼开局和动态、发回复、结束话题和空闲清理。

use std::time::Instant;

use wonton_l10n::Arg;

use super::{
    Job, JobKind, PromptFinished, Room, RoomState, TurnContext, format_error, now, permissions,
    routing, sessions,
};
use crate::acp::PromptResult;
use crate::commands;
use crate::config::Limits;
use crate::error::{Detail, Error, Result};
use crate::l10n::text;
use crate::matrix::IncomingText;
use crate::topics::{SessionKey, Topics, TurnStart};

/// 发到这条消息所在的地方：主对话，或它所属的话题。
pub(super) async fn send_to<R: Room>(
    context: &TurnContext<R>,
    key: &SessionKey,
    text: &str,
) -> Result<()> {
    match key {
        SessionKey::Main => context.room.send_text(text).await,
        SessionKey::Topic(root) => context.room.send_thread_text(root, text).await,
    }
}

/// 一条主人消息进来了。
pub(super) async fn receive_job<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    incoming: IncomingText,
) -> Result<()> {
    let key = routing::session_key(incoming.thread_root.as_deref());
    if incoming.text.is_empty() {
        return send_to(context, &key, &text(context.lang, "input-unsupported", &[])).await;
    }
    if commands::is_help_request(&incoming.text) {
        return send_to(context, &key, &commands::help(context.lang)).await;
    }
    if state.sessions.existing(&key).is_none()
        && let Some(root) = &incoming.thread_root
    {
        let (root_text, preceding) =
            routing::opening_context(state, root, &incoming.event_id, &incoming.text);
        let session = state.sessions.state(&key);
        session.root_text = root_text;
        session.preceding = preceding;
    }
    let busy = state
        .sessions
        .existing(&key)
        .is_some_and(|state| state.active);
    if context
        .login
        .receive(
            &context.room,
            context.group,
            incoming.thread_root.as_deref(),
            &incoming,
            busy,
            context.lang,
        )
        .await?
    {
        return Ok(());
    }
    let job = match parse_job(&incoming.text, &context.bridge.config.limits) {
        Ok(job) => job,
        Err(error) => return send_to(context, &key, &format_error(&error, context.lang)).await,
    };
    if job.kind == JobKind::End {
        return end(context, state, topics, &key).await;
    }
    match job.kind {
        JobKind::Interrupt if busy => conclude_interrupt(context, state, &key, job).await,
        JobKind::Steer if busy && context.bridge.acp.capabilities().steering => {
            steer(context, state, &key, job).await
        }
        _ if busy => queue(context, state, &key, job).await,
        _ => start_or_report(context, state, topics, key, job).await,
    }
}

/// `/wonton interrupt`：取消当前一轮，等它的应答回来再发新的。
async fn conclude_interrupt<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    key: &SessionKey,
    job: Job,
) -> Result<()> {
    let session = state.sessions.state(key);
    if session.pending_interrupt.is_some() {
        return send_to(context, key, &text(context.lang, "interrupt-pending", &[])).await;
    }
    session.pending_interrupt = Some(job);
    let Some(session_id) = session.session_id.clone() else {
        session.pending_interrupt = None;
        return send_to(
            context,
            key,
            &format_error(
                &Error::AgentNotReady(Detail::msg("no-active-session")),
                context.lang,
            ),
        )
        .await;
    };
    if let Err(error) = context.bridge.acp.cancel(&session_id).await {
        state.sessions.state(key).pending_interrupt = None;
        return send_to(context, key, &format_error(&error, context.lang)).await;
    }
    Ok(())
}

/// `/wonton steer`：Agent 支持时直接插进进行中的一轮，不支持返回 promptRequired 时排队。
async fn steer<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    key: &SessionKey,
    job: Job,
) -> Result<()> {
    let Some(session_id) = state
        .sessions
        .existing(key)
        .and_then(|session| session.session_id.clone())
    else {
        return send_to(
            context,
            key,
            &format_error(
                &Error::AgentNotReady(Detail::msg("no-active-session")),
                context.lang,
            ),
        )
        .await;
    };
    let outcome = match context.bridge.acp.steer(&session_id, &job.text).await {
        Ok(outcome) => outcome,
        Err(error) => return send_to(context, key, &format_error(&error, context.lang)).await,
    };
    match outcome.as_str() {
        "injected" => Ok(()),
        "promptRequired" => {
            queue(
                context,
                state,
                key,
                Job {
                    kind: JobKind::Normal,
                    ..job
                },
            )
            .await
        }
        _ => send_to(context, key, &text(context.lang, "steer-rejected", &[])).await,
    }
}

async fn queue<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    key: &SessionKey,
    job: Job,
) -> Result<()> {
    let session = state.sessions.state(key);
    if session.queue.len() >= context.bridge.config.limits.max_queue {
        let error = Error::Busy(Detail::msg("queue-full"));
        return send_to(context, key, &format_error(&error, context.lang)).await;
    }
    session.queue.push_back(job);
    Ok(())
}

fn parse_job(input: &str, limits: &Limits) -> Result<Job> {
    if input.len() > limits.max_input_bytes {
        return Err(Error::InvalidInput(Detail::msg_with(
            "input-too-long",
            [("max", limits.max_input_bytes.to_string())],
        )));
    }
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidInput(Detail::msg("input-empty")));
    }
    if trimmed == "/wonton end" {
        return Ok(Job {
            kind: JobKind::End,
            text: String::new(),
        });
    }
    let (kind, body) = if let Some(body) = trimmed.strip_prefix("/wonton interrupt ") {
        (JobKind::Interrupt, body)
    } else if let Some(body) = trimmed.strip_prefix("/wonton queue ") {
        (JobKind::Queue, body)
    } else if let Some(body) = trimmed.strip_prefix("/wonton steer ") {
        (JobKind::Steer, body)
    } else {
        (JobKind::Normal, trimmed)
    };
    if body.trim().is_empty() {
        return Err(Error::InvalidInput(Detail::msg("control-without-text")));
    }
    Ok(Job {
        kind,
        text: body.trim().to_owned(),
    })
}

async fn start_or_report<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    key: SessionKey,
    job: Job,
) -> Result<()> {
    if let Err(error) = start_job(context, state, topics, key.clone(), job).await {
        send_to(context, &key, &format_error(&error, context.lang)).await?;
    }
    Ok(())
}

/// 开始一轮：开局说明或动态拼在用户这句话前面，登记工作板后发给 Agent。
async fn start_job<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    key: SessionKey,
    job: Job,
) -> Result<()> {
    let session = state.sessions.state(&key);
    let first = session.first_turn;
    if first && key.is_topic() && session.root_text.is_none() {
        // 话题根不在历史里（桥重启过、或者根本就是这条消息自己开的头）：拿这句话当起点。
        session.root_text = Some(job.text.clone());
    }
    // 只有话题第一轮带开局说明；之后每轮只带别人的动态。任务话题不注入别人的动态
    // （编排器自己分工），第一轮的开局说明照带。
    let prefix = if first || topics.is_task(&key) {
        session
            .root_text
            .as_ref()
            .map(|root| topics.opening_for(&key, root, &session.preceding, context.lang))
    } else {
        topics.digest_for(&key, context.lang)
    };
    if topics
        .turn_started(&key, &job.text, now(), context.lang)
        .await?
        == TurnStart::Queued
    {
        return announce_queue(context, state, topics, &key).await;
    }
    let session = state.sessions.state(&key);
    session.told_queued = false;
    let prepared = sessions::ensure_session(
        &context.bridge,
        &context.ask_tx,
        &context.room.room_id(),
        &key,
        session,
    )
    .await;
    let (session_id, lost, created) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            let _ = topics.turn_finished(&key, "", now(), context.lang).await;
            return Err(error);
        }
    };
    state.sessions.bind(&key, &session_id);
    state.sessions.state(&key).first_turn = false;
    if lost {
        send_to(context, &key, &text(context.lang, "session-lost-new", &[])).await?;
    }
    if first && created == Some(false) {
        send_to(context, &key, &text(context.lang, "topic-no-ask-mode", &[])).await?;
    }
    let prompt = match prefix {
        Some(prefix) => format!("{prefix}\n\n{}", job.text),
        None => job.text,
    };
    state.sessions.state(&key).active = true;
    state.last_activity = Instant::now();
    let acp = context.bridge.acp.clone();
    let room = context.room.clone();
    let limits = context.bridge.config.limits.clone();
    let finished_tx = context.finished_tx.clone();
    let thread = key.clone();
    let lang = context.lang;
    tokio::spawn(async move {
        let _ = room.typing(true).await;
        let result = acp
            .prompt(
                &session_id,
                &prompt,
                limits.max_output_bytes,
                limits.turn_timeout,
                lang,
            )
            .await;
        let _ = room.typing(false).await;
        let _ = finished_tx
            .send(PromptFinished {
                key: thread,
                result,
            })
            .await;
    });
    Ok(())
}

/// 活跃话题已满时在话题里写明在排，同一话题只提醒一次。
async fn announce_queue<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &Topics,
    key: &SessionKey,
) -> Result<()> {
    let session = state.sessions.state(key);
    if session.told_queued {
        return Ok(());
    }
    session.told_queued = true;
    let ahead = topics.active_topics();
    send_to(
        context,
        key,
        &text(
            context.lang,
            "topic-queued",
            &[("ahead", Arg::from(ahead.to_string()))],
        ),
    )
    .await
}

/// 一轮结束：记结论、发 notice、放行等权限的请求、跑队列里的下一句。
pub(super) async fn finished<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    finished: PromptFinished,
) -> Result<()> {
    let key = finished.key;
    state.sessions.state(&key).active = false;
    state.last_activity = Instant::now();
    let reply = match &finished.result {
        Ok(result) => result.text.clone(),
        Err(_) => String::new(),
    };
    if let Some(notice) = topics
        .turn_finished(&key, &reply, now(), context.lang)
        .await?
    {
        // 主对话里一行不推送的提示；点开进话题。
        let _ = context.room.send_notice(&notice).await;
    }
    permissions::release(context, state, topics, &key).await?;
    permissions::drop_waiting(state, &key);
    report_prompt(context, &key, finished.result).await;
    if topics.is_finished(&key) {
        let _ = sessions::close_session(&context.bridge, state, &key).await;
        return promote(context, state, topics).await;
    }
    let next = state
        .sessions
        .state(&key)
        .pending_interrupt
        .take()
        .or_else(|| state.sessions.state(&key).queue.pop_front());
    if let Some(job) = next {
        start_or_report(context, state, topics, key, job).await?;
    }
    Ok(())
}

async fn report_prompt<R: Room>(
    context: &TurnContext<R>,
    key: &SessionKey,
    result: Result<PromptResult>,
) {
    let lang = context.lang;
    let line = match result {
        Ok(result) => {
            if let Some(message) = result.permission_error {
                text(
                    lang,
                    "permission-failed",
                    &[("message", Arg::from(message.format(lang)))],
                )
            } else if result.permissions_cancelled > 0 {
                text(lang, "permissions-cancelled", &[])
            } else if result.output_limited {
                text(lang, "output-limited", &[])
            } else if result.stop_reason != "cancelled" && result.text.trim().is_empty() {
                text(
                    lang,
                    "no-text",
                    &[("stop_reason", Arg::from(result.stop_reason.as_str()))],
                )
            } else if result.stop_reason != "cancelled" {
                result.text
            } else {
                return;
            }
        }
        Err(error) => format_error(&error, lang),
    };
    let _ = send_to(context, key, &line).await;
}

/// 话题空闲判断、主对话空闲关闭。定时器每 30 秒调一次。
pub(super) async fn idle<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
) -> Result<()> {
    let main = SessionKey::Main;
    let idle_timeout = context.bridge.config.limits.idle_timeout;
    let close_main = state.last_activity.elapsed() >= idle_timeout
        && state
            .sessions
            .existing(&main)
            .is_some_and(|session| !session.active && session.session_id.is_some());
    if close_main {
        match sessions::close_session(&context.bridge, state, &main).await {
            Ok(()) => state.last_activity = Instant::now(),
            Err(error) => {
                send_to(context, &main, &format_error(&error, context.lang)).await?;
            }
        }
    }
    let ended = topics
        .sweep(now(), context.bridge.config.limits.topic_idle, context.lang)
        .await?;
    for key in &ended {
        let _ = sessions::close_session(&context.bridge, state, key).await;
        permissions::release(context, state, topics, key).await?;
        permissions::drop_waiting(state, key);
        let _ = send_to(context, key, &text(context.lang, "topic-idle-ended", &[])).await;
    }
    if !ended.is_empty() {
        promote(context, state, topics).await?;
    }
    Ok(())
}

/// `/wonton end`：结束话题。这一轮还在跑就等它结束再关会话。
async fn end<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    key: &SessionKey,
) -> Result<()> {
    if !key.is_topic() {
        return send_to(
            context,
            key,
            &text(context.lang, "topic-main-cannot-end", &[]),
        )
        .await;
    }
    if !topics.end(key, now(), context.lang).await? {
        return send_to(
            context,
            key,
            &text(context.lang, "topic-already-ended", &[]),
        )
        .await;
    }
    let session = state.sessions.state(key);
    session.queue.clear();
    session.pending_interrupt = None;
    let active = session.active;
    if !active {
        let _ = sessions::close_session(&context.bridge, state, key).await;
    }
    permissions::release(context, state, topics, key).await?;
    permissions::drop_waiting(state, key);
    let id = if active {
        "topic-ended-running"
    } else {
        "topic-ended"
    };
    send_to(context, key, &text(context.lang, id, &[])).await?;
    promote(context, state, topics).await
}

/// 有名额就放排队的下一个话题进来。
pub(super) async fn promote<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
) -> Result<()> {
    while let Some((key, prompt)) = topics.poll_admission(now()).await? {
        start_or_report(
            context,
            state,
            topics,
            key,
            Job {
                kind: JobKind::Normal,
                text: prompt,
            },
        )
        .await?;
    }
    Ok(())
}
