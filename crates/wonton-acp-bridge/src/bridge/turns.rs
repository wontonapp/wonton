//! 收消息、跑一轮、收尾：分流到会话、拼开局和动态、发回复、结束话题和空闲清理。

use std::time::Instant;

use wonton_l10n::Arg;

use wonton_protocol::SideChatResultState;

use super::{
    Job, JobKind, PromptFinished, Room, RoomState, TurnContext, attachments, busy, chain,
    format_error, now, permissions, routing, sessions, side_chats,
};
use crate::acp::PromptResult;
use crate::commands;
use crate::config::Limits;
use crate::error::{Detail, Error, Result};
use crate::l10n::text;
use crate::matrix::{Incoming, IncomingContent};
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
    incoming: Incoming,
) -> Result<()> {
    if let IncomingContent::SideChatRoot { title } = &incoming.content {
        // 旁聊的根不是输入（ADR-0093 §1）：记下标题给话题取名用，不回应。
        state.side_chats.observe(&incoming.event_id, title.clone());
        return Ok(());
    }
    let key = routing::session_key(incoming.thread_root.as_deref());
    if let IncomingContent::Unsupported { kind } = &incoming.content {
        return send_to(
            context,
            &key,
            &text(
                context.lang,
                "input-unsupported",
                &[("kind", Arg::from(kind.clone()))],
            ),
        )
        .await;
    }
    let user_text = match &incoming.content {
        IncomingContent::Media(media) => media.caption.clone().unwrap_or_default(),
        IncomingContent::Text(text) => text.clone(),
        IncomingContent::Unsupported { .. } | IncomingContent::SideChatRoot { .. } => String::new(),
    };
    if matches!(&incoming.content, IncomingContent::Text(text) if text.is_empty()) {
        return send_to(
            context,
            &key,
            &text(
                context.lang,
                "input-unsupported",
                &[("kind", Arg::from("other"))],
            ),
        )
        .await;
    }
    if matches!(&incoming.content, IncomingContent::Text(text) if commands::is_help_request(text)) {
        return send_to(context, &key, &commands::help(context.lang)).await;
    }
    if state.sessions.existing(&key).is_none()
        && let Some(root) = &incoming.thread_root
    {
        let (root_text, preceding) =
            routing::opening_context(state, root, &incoming.event_id, &user_text);
        let session = state.sessions.state(&key);
        session.root_text = root_text;
        session.preceding = preceding;
    }
    let busy = state
        .sessions
        .existing(&key)
        .is_some_and(|state| state.active);
    // 登录流程只认文字：发码、回授权码都是打字，不收附件。
    if matches!(incoming.content, IncomingContent::Text(_))
        && context
            .login
            .receive(
                &context.room,
                context.group,
                incoming.thread_root.as_deref(),
                &user_text,
                &incoming.event_id,
                busy,
                context.lang,
            )
            .await?
    {
        return Ok(());
    }
    let job = match incoming.content {
        // 旁聊的根在上面已经处理过，这里是保险。
        IncomingContent::SideChatRoot { .. } => return Ok(()),
        IncomingContent::Text(text) => parse_job(
            &incoming.event_id,
            &incoming.sender,
            &text,
            &context.bridge.config.limits,
        ),
        IncomingContent::Media(media) => {
            media_job(context, media, &incoming.event_id, &incoming.sender).await
        }
        IncomingContent::Unsupported { .. } => return Ok(()),
    };
    let job = match job {
        Ok(job) => job,
        Err(error) => return send_to(context, &key, &format_error(&error, context.lang)).await,
    };
    if job.kind == JobKind::End {
        return end(context, state, topics, &key).await;
    }
    match job.kind {
        JobKind::Interrupt if busy => busy::interrupt(context, state, &key, job).await,
        _ if busy => busy::receive(context, state, &key, job).await,
        _ => start_or_report(context, state, topics, key, vec![job]).await,
    }
}

/// 媒体消息的任务：现在就下载落工作区，失败按类别报错；之后排队（含话题准入）只带
/// 已落盘的路径，不会在中途丢。说明文字也是用户输入，受同一个输入上限。
async fn media_job<R: Room>(
    context: &TurnContext<R>,
    media: crate::matrix::MediaAttachment,
    event_id: &str,
    sender: &str,
) -> Result<Job> {
    if media
        .caption
        .as_ref()
        .is_some_and(|caption| caption.len() > context.bridge.config.limits.max_input_bytes)
    {
        return Err(too_long(&context.bridge.config.limits));
    }
    let prepared = attachments::prepare(
        &*context.room,
        &media,
        event_id,
        &context.bridge.config.workspace,
    )
    .await?;
    Ok(Job {
        kind: JobKind::Normal,
        text: media.caption.unwrap_or_default(),
        event_id: event_id.to_owned(),
        sender: sender.to_owned(),
        media: Some(prepared),
    })
}

fn too_long(limits: &Limits) -> Error {
    Error::InvalidInput(Detail::msg_with(
        "input-too-long",
        [("max", limits.max_input_bytes.to_string())],
    ))
}

/// 一轮（或一次插话）的内容块：一个文字块在前，附件在后；没有文字就只有附件块。文字是说明（开局说明或
/// 动态）加上各条消息的文字，按到达顺序空一行接起来：攒着的几条合成一轮时，Agent 看到的就是主人
/// 连着说的几句话。
pub(super) async fn prompt_blocks<R: Room>(
    context: &TurnContext<R>,
    prefix: Option<String>,
    jobs: &[Job],
) -> Result<Vec<serde_json::Value>> {
    let text = prefix
        .into_iter()
        .chain(jobs.iter().map(|job| job.text.trim().to_owned()))
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let text = text.trim_end().to_owned();
    let mut blocks = Vec::new();
    if !text.is_empty() {
        blocks.push(attachments::text_block(&text));
    }
    for media in jobs.iter().filter_map(|job| job.media.as_ref()) {
        blocks.extend(
            attachments::blocks(
                media,
                context.bridge.acp.capabilities().prompt_image,
                &context.bridge.config.workspace,
            )
            .await?,
        );
    }
    Ok(blocks)
}

fn parse_job(event_id: &str, sender: &str, input: &str, limits: &Limits) -> Result<Job> {
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
            event_id: event_id.to_owned(),
            sender: sender.to_owned(),
            media: None,
        });
    }
    let (kind, body) = match trimmed.strip_prefix("/wonton interrupt") {
        Some(body) if body.is_empty() || body.starts_with(char::is_whitespace) => {
            (JobKind::Interrupt, body)
        }
        _ => (JobKind::Normal, trimmed),
    };
    if body.trim().is_empty() {
        return Err(Error::InvalidInput(Detail::msg("control-without-text")));
    }
    Ok(Job {
        kind,
        text: body.trim().to_owned(),
        event_id: event_id.to_owned(),
        sender: sender.to_owned(),
        media: None,
    })
}

async fn start_or_report<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    key: SessionKey,
    batch: Vec<Job>,
) -> Result<()> {
    if let Err(error) = start_job(context, state, topics, key.clone(), batch).await {
        send_to(context, &key, &format_error(&error, context.lang)).await?;
    }
    Ok(())
}

/// 开始一轮：开局说明或动态拼在用户的话前面，登记工作板后发给 Agent。`batch` 是这一轮要交给 Agent 的
/// 消息，按到达顺序：平时一条，上一轮跑着时攒下的是几条合成一轮（`busy.rs`）。
async fn start_job<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    key: SessionKey,
    batch: Vec<Job>,
) -> Result<()> {
    let Some(last) = batch.last() else {
        return Ok(());
    };
    let display = batch
        .iter()
        .map(Job::display)
        .collect::<Vec<_>>()
        .join("\n");
    let session = state.sessions.state(&key);
    let first = session.first_turn;
    if first && key.is_topic() && session.root_text.is_none() {
        // 话题根不在历史里（桥重启过、或者根本就是这条消息自己开的头）：拿这句话当起点。
        session.root_text = Some(display.clone());
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
    // 话题准入排队只发生在话题的第一句（这时还没有在跑的一轮，`batch` 只有一条）；攒下的几条
    // 是一轮结束后才发，话题已经入场，不会在这里排队，所以排队位只存一条的附件就够。
    let started = topics
        .turn_started(
            &key,
            &display,
            &last.event_id,
            &last.sender,
            last.media.as_ref(),
            now(),
            context.lang,
        )
        .await?;
    // 话题的名字按 ADR-0093 §1 定：根消息带 `app.wonton.side_chat.title` 用它，否则根
    // 正文第一行的前 20 个字符；根不在历史里（桥重启过）时是第一句话，App 的标题正是
    // 按它取的。任务话题的名字由编排器给，不改。
    if first
        && !topics.is_task(&key)
        && let SessionKey::Topic(root) = &key
    {
        let root_body = state
            .sessions
            .state(&key)
            .root_text
            .clone()
            .unwrap_or_else(|| display.clone());
        let title = side_chats::title_of(&state.side_chats, root, &root_body);
        topics.set_name(&key, &title).await?;
    }
    if started == TurnStart::Queued {
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
    // 一轮的内容块：说明（含开局说明/动态）在前，附件在后；没有文字就只有附件块。
    // 块在领链前拼好：附件转换出错时这轮还没领链，不用报 End。
    let blocks = prompt_blocks(context, prefix, &batch).await?;
    // 领链（ADR-0061 第 10 节）：拿到链才发给 Agent；被拒或核心不可用就在房间里
    // 按类别说明，不开轮，也不发已读回执。
    if !chain::begin(
        &context.bridge.chains,
        context.room.as_ref(),
        context.lang,
        &key,
        last,
        &session_id,
    )
    .await
    {
        let _ = topics.turn_finished(&key, "", now(), context.lang).await;
        return Ok(());
    }
    launch_prompt(context, state, &key, &session_id, blocks, last).await;
    Ok(())
}

/// 领链通过后把这一轮发给 Agent：标记在跑、挂状态消息（ADR-0082）、发已读回执，
/// 提示词在后台任务里发，结束后报 End。
async fn launch_prompt<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    key: &SessionKey,
    session_id: &str,
    blocks: Vec<serde_json::Value>,
    trigger: &Job,
) {
    state.sessions.state(key).active = true;
    state.last_activity = Instant::now();
    // 一轮一条的状态消息（ADR-0082）：发出提示词时先挂上“在想”，之后随 ACP 事件编辑。
    state
        .run_status
        .start(context.room.as_ref(), key, &trigger.event_id, context.lang)
        .await;
    // 这一轮就要发给 Agent：这批消息算收到了。回执只往前走，发给最后一条就覆盖了前面几条。
    busy::receipt(context, &trigger.event_id).await;
    let acp = context.bridge.acp.clone();
    let chains = context.bridge.chains.clone();
    let room = context.room.clone();
    let limits = context.bridge.config.limits.clone();
    let finished_tx = context.finished_tx.clone();
    let thread = key.clone();
    let session_id = session_id.to_owned();
    let lang = context.lang;
    tokio::spawn(async move {
        let _ = room.typing(true).await;
        let result = acp
            .prompt(
                &session_id,
                blocks,
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
        // 一轮结束（成功、取消、出错）都报 End（ADR-0061 第 10 节）；放回执后面，
        // 房间里的收尾不等核心。
        chains.end(&session_id).await;
    });
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

/// 一轮结束：记结论、发 notice、放行等权限的请求，把这一轮跑着时攒下的消息合成下一轮。
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
    if let Some(title) = topics
        .turn_finished(&key, &reply, now(), context.lang)
        .await?
        && let SessionKey::Topic(root) = &key
    {
        // 主对话里一行不推送的结果行（ADR-0093）：带话题根和状态，App 点开进旁聊。
        let turn_state = side_chats::result_state(&finished.result);
        side_chats::result_line(
            context.room.as_ref(),
            root,
            &title,
            turn_state,
            context.lang,
        )
        .await;
    }
    permissions::release(context, state, topics, &key).await?;
    permissions::drop_waiting(state, &key);
    // ADR-0082：先把答案发出去，再把状态消息编辑成 done/failed，时间线上状态在上、
    // 答案在下。
    report_prompt(context, &key, &finished.result).await;
    state
        .run_status
        .finish(context.room.as_ref(), &key, &finished.result)
        .await;
    let held = busy::take_held(state, &key);
    if topics.is_finished(&key) {
        let _ = sessions::close_session(&context.bridge, state, &key).await;
        return promote(context, state, topics).await;
    }
    if !held.is_empty() {
        start_or_report(context, state, topics, key, held).await?;
    }
    Ok(())
}

async fn report_prompt<R: Room>(
    context: &TurnContext<R>,
    key: &SessionKey,
    result: &Result<PromptResult>,
) {
    let lang = context.lang;
    let line = match result {
        Ok(result) => {
            if let Some(message) = result.permission_error.as_ref() {
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
                result.text.clone()
            } else {
                return;
            }
        }
        Err(error) => format_error(error, lang),
    };
    let _ = send_to(context, key, &line).await;
}

/// 话题空闲判断、主对话空闲关闭。定时器每 30 秒调一次。
pub(super) async fn idle<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
) -> Result<()> {
    if state.last_sweep.elapsed() >= attachments::SWEEP_INTERVAL {
        state.last_sweep = Instant::now();
        attachments::sweep(&context.bridge.config.workspace).await;
    }
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
        // 旁聊关了，主对话里发 ended 的结果行（ADR-0093 §1）。
        if let SessionKey::Topic(root) = key {
            side_chats::result_line(
                context.room.as_ref(),
                root,
                &topics.name(key),
                SideChatResultState::Ended,
                context.lang,
            )
            .await;
        }
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
    session.buffer.clear();
    session.cancelling = false;
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
    // 旁聊关了，主对话里发 ended 的结果行（ADR-0093 §1）。
    if let SessionKey::Topic(root) = key {
        side_chats::result_line(
            context.room.as_ref(),
            root,
            &topics.name(key),
            SideChatResultState::Ended,
            context.lang,
        )
        .await;
    }
    promote(context, state, topics).await
}

/// 有名额就放排队的下一个话题进来。
pub(super) async fn promote<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
) -> Result<()> {
    while let Some((key, prompt, event_id, sender, media)) = topics.poll_admission(now()).await? {
        start_or_report(
            context,
            state,
            topics,
            key,
            vec![Job {
                kind: JobKind::Normal,
                text: prompt,
                event_id,
                sender,
                media,
            }],
        )
        .await?;
    }
    Ok(())
}
