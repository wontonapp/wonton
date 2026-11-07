use std::{
    collections::{HashSet, VecDeque},
    result::Result as StdResult,
    sync::Arc,
    time::Instant,
};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use tokio::sync::{broadcast, mpsc};
use wonton_l10n::Lang;
use wonton_matrix::{
    ChatRoom, MatrixSession, Message, OwnedUserId, SessionConfig, StoreConfig, UserId,
};

use crate::{
    acp::{AcpClient, AgentNotice, PromptResult},
    commands::{self, CommandSink, Commands},
    config::Config,
    error::{Detail, Error, Result as BridgeResult, format_error},
    job::{Job, JobKind, parse_error, parse_job},
    l10n::text,
    locale::{CoreMcp, Locales},
    login::{LoginChat, LoginEvent, LoginFlow, LoginSink},
    matrix::{IncomingText, new_owner_messages},
    room_owner::room_owner,
    state::{SessionRecord, SessionStore},
};

#[derive(Clone)]
struct Bridge {
    config: Config,
    matrix: Arc<MatrixSession>,
    acp: Arc<AcpClient>,
    sessions: Arc<SessionStore>,
    bot: OwnedUserId,
    locales: Arc<Locales<CoreMcp>>,
}

impl Bridge {
    /// 这个房间说话用的语言（按房间主人，取不到照兜底退）。
    async fn lang(&self, room_owner: &UserId) -> Lang {
        self.locales.lang(room_owner).await
    }
}

struct PromptFinished {
    result: BridgeResult<PromptResult>,
}

struct RoomSink<'a> {
    room: &'a ChatRoom,
    bot: &'a UserId,
}

impl CommandSink for RoomSink<'_> {
    async fn set_commands(&self, content: &serde_json::Value) -> wonton_matrix::Result<()> {
        self.room
            .send_state_event_raw(
                wonton_protocol::BOT_COMMANDS_EVENT,
                self.bot.as_str(),
                content.clone(),
            )
            .await
    }

    async fn send_command_text(&self, text: &str) -> wonton_matrix::Result<()> {
        self.room.send_text(text).await
    }
}

impl LoginSink for RoomSink<'_> {
    async fn send_login_text(&self, text: &str) -> BridgeResult<()> {
        Ok(self.room.send_text(text).await?)
    }
    async fn redact(&self, event_id: &str, reason: &str) -> BridgeResult<()> {
        Ok(self.room.redact(event_id, reason).await?)
    }
}

/// 登录是全 Agent 一份的：群里发起的命令转到和主人的私聊，事件也只在私聊里回。
struct LoginService {
    flow: tokio::sync::Mutex<LoginFlow>,
    /// 主人和 Bot 的私聊；主房间本身是私聊时为 None。
    direct: Option<Arc<ChatRoom>>,
    bot: wonton_matrix::OwnedUserId,
}

impl LoginService {
    // 房间、群标记、消息、ACP、socket、忙状态、语言互相独立，再包一层也是逐个填。
    #[allow(clippy::too_many_arguments)]
    async fn receive(
        &self,
        room: &Arc<ChatRoom>,
        group: bool,
        incoming: &IncomingText,
        acp: &Arc<AcpClient>,
        socket: &std::path::Path,
        busy: bool,
        lang: Lang,
    ) -> BridgeResult<bool> {
        let bot = &self.bot;
        let current = RoomSink { room, bot };
        let direct = self.direct.as_ref().map(|room| RoomSink { room, bot });
        self.flow
            .lock()
            .await
            .receive_from(
                &incoming.text,
                &incoming.event_id,
                &LoginChat {
                    current: &current,
                    direct: direct.as_ref(),
                    group,
                },
                acp,
                socket,
                busy,
                lang,
            )
            .await
    }

    async fn event(&self, event: LoginEvent, room: &ChatRoom, lang: Lang) -> BridgeResult<bool> {
        self.flow
            .lock()
            .await
            .event(
                event,
                &RoomSink {
                    room,
                    bot: &self.bot,
                },
                lang,
            )
            .await
    }
}

struct RoomState {
    owner: OwnedUserId,
    session_id: Option<String>,
    loaded: bool,
    active: bool,
    queue: VecDeque<Job>,
    pending_interrupt: Option<Job>,
    last_activity: Instant,
}

pub async fn run() -> Result<()> {
    let config = Config::from_env()?;
    tokio::fs::create_dir_all(&config.bridge_data)
        .await
        .context("创建桥数据目录")?;
    let matrix = MatrixSession::login_with_access_token(
        &SessionConfig {
            homeserver_url: config.homeserver.clone(),
            device_name: "Wonton ACP bridge".into(),
            store: StoreConfig::Sqlite {
                path: config.matrix_store.clone(),
                passphrase: None,
            },
            proxy: None,
        },
        &config.access_token,
    )
    .await
    .context("登录 Matrix")?;
    matrix.start_sync().await.context("启动 Matrix 同步")?;
    let bot = matrix.user_id().to_owned();
    let room = Arc::new(
        matrix
            .watch_room(&config.home_room)
            .await
            .context("打开 Matrix 主房间")?,
    );
    // 主房间不是和主人的私聊（共享 Bot 的主房间在家人群）时，登录链接改到私聊里发。
    let direct = if is_group(&matrix, &room, &config.owner).await {
        watch_direct(&matrix, &config.owner).await
    } else {
        None
    };
    let (login, login_rx) = LoginFlow::new();
    let login = Arc::new(LoginService {
        flow: tokio::sync::Mutex::new(login),
        direct: direct.clone(),
        bot: bot.clone(),
    });
    let acp = connect_agent(&config).await?;
    let sessions = Arc::new(SessionStore::open(config.bridge_data.join("sessions.json")).await?);
    let locales = Arc::new(Locales::new(
        CoreMcp::new(&config.mcp_url),
        config.owner.clone(),
    ));
    let bridge = Bridge {
        config,
        matrix: Arc::new(matrix),
        acp: Arc::new(acp),
        sessions,
        bot,
        locales,
    };
    match direct {
        Some(direct) => {
            // 登录事件只由私聊循环消费；它结束后整座桥退出重连。
            let mut direct_loop = tokio::spawn(run_room(
                bridge.clone(),
                direct,
                Some(login_rx),
                login.clone(),
            ));
            let result = tokio::select! {
                result = run_room(bridge, room, None, login) => result,
                result = &mut direct_loop => match result {
                    Ok(result) => result,
                    Err(error) => Err(join_error(error)),
                },
            };
            direct_loop.abort();
            result.map_err(|error| anyhow::anyhow!(error))
        }
        None => run_room(bridge, room, Some(login_rx), login)
            .await
            .map_err(|error| anyhow::anyhow!(error)),
    }
}

/// 当前房间不是和主人的私聊。私聊只有 `m.direct` 里那一个，群是别的房间；
/// 私聊标记还没同步到时退回成员数，取不到按私聊。
async fn is_group(matrix: &MatrixSession, room: &ChatRoom, owner: &wonton_matrix::UserId) -> bool {
    match matrix.direct_chat_with(owner) {
        Some(direct) => direct != *room.room_id(),
        None => match room.joined_member_count().await {
            Ok(count) => count > 2,
            Err(error) => {
                tracing::warn!(error = %error, "取不到房间成员数，按私聊处理");
                false
            }
        },
    }
}

/// 等和主人的私聊同步进来（首次安装后可能还没同步到），拿不到就当作没有。
async fn watch_direct(
    matrix: &MatrixSession,
    owner: &wonton_matrix::UserId,
) -> Option<Arc<ChatRoom>> {
    for _ in 0..5 {
        if let Some(room_id) = matrix.direct_chat_with(owner) {
            match matrix.watch_room(&room_id).await {
                Ok(room) => return Some(Arc::new(room)),
                Err(error) => tracing::warn!(error = %error, "打开和主人的私聊失败，重试"),
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    None
}

fn join_error(error: tokio::task::JoinError) -> Error {
    Error::TransportUnavailable(Detail::msg_with(
        "direct-task-failed",
        [("error", error.to_string())],
    ))
}

async fn next_login_event(events: &mut Option<mpsc::Receiver<LoginEvent>>) -> Option<LoginEvent> {
    match events {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

async fn connect_agent(config: &Config) -> BridgeResult<AcpClient> {
    let broker = crate::permission::PermissionBroker::new(&config.mcp_url)?;
    let deadline = Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let attempt = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            AcpClient::connect(&config.socket, broker.clone()),
        )
        .await
        .unwrap_or_else(|_| Err(Error::AgentNotReady(Detail::msg("agent-init-timeout"))));
        match attempt {
            Ok(client) => return Ok(client),
            Err(error) if Instant::now() < deadline => {
                tracing::warn!(error = %error, "ACP Agent 未就绪，重试");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

async fn run_room(
    bridge: Bridge,
    room: Arc<ChatRoom>,
    mut login_events: Option<mpsc::Receiver<LoginEvent>>,
    login: Arc<LoginService>,
) -> BridgeResult<()> {
    let bot = bridge.bot.clone();
    let owner = room_owner(room.as_ref(), &bot, &bridge.config.owner).await;
    let mut start_lang = bridge.lang(&owner).await;
    // 连上 Agent 前先摆出桥自己的命令，Agent 可能一直不报命令表。
    let mut commands = Commands::new();
    commands
        .reset(
            &RoomSink {
                room: room.as_ref(),
                bot: &bot,
            },
            start_lang,
        )
        .await;
    let mut updates = room.message_updates().await;
    let mut acp_notices = bridge.acp.subscribe();
    let mut acp_closed = bridge.acp.closed();
    if *acp_closed.borrow() {
        return stop_for_agent_disconnect(&room, &mut commands, &bot, start_lang).await;
    }
    let initial: Vec<Message> = updates.next().await.unwrap_or_default();
    let mut seen = initial
        .iter()
        .filter_map(|message| message.event_id.as_ref().map(ToString::to_string))
        .collect::<HashSet<_>>();
    let (finished_tx, mut finished_rx) = mpsc::channel(16);
    let mut state = RoomState {
        owner,
        session_id: bridge
            .sessions
            .get(room.room_id().as_str())
            .await
            .map(|record| record.session_id),
        loaded: false,
        active: false,
        queue: VecDeque::new(),
        pending_interrupt: None,
        last_activity: Instant::now(),
    };
    let mut idle_tick = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        tokio::select! {
            Some(messages) = updates.next() => {
                route_owner_messages(
                    &bridge,
                    &room,
                    &mut state,
                    &login,
                    &messages,
                    &mut seen,
                    &finished_tx,
                )
                .await?;
            }
            notice = acp_notices.recv() => {
                start_lang = bridge.lang(&state.owner).await;
                handle_notice(
                    &mut commands,
                    &room,
                    &bot,
                    state.session_id.as_deref(),
                    notice,
                    start_lang,
                )
                .await;
            }
            Some(finished) = finished_rx.recv() => {
                finished_job(&bridge, &room, &mut state, finished, &finished_tx).await?;
            }
            Some(event) = next_login_event(&mut login_events) => {
                let lang = bridge.lang(&state.owner).await;
                if login.event(event, room.as_ref(), lang).await? {
                    // 终端登录成功后重开 ACP 连接：进程退出码 0，Compose 拉起后重新 initialize。
                    room.send_text(&text(lang, "login-reconnect", &[])).await?;
                    return Ok(());
                }
            }
            _ = idle_tick.tick() => {
                if !state.active
                    && state.last_activity.elapsed() >= bridge.config.limits.idle_timeout
                {
                    close_idle_session(&bridge, &room, &mut state).await?;
                }
            }
            changed = acp_closed.changed() => {
                if changed.is_ok() && *acp_closed.borrow() {
                    return stop_for_agent_disconnect(
                        &room,
                        &mut commands,
                        &bot,
                        bridge.lang(&state.owner).await,
                    )
                    .await;
                }
            }
            else => {
                return Err(Error::TransportUnavailable(Detail::msg("stream-closed")))
            }
        }
    }
}

/// 一轮结束：先回话，再接着跑排队或插队的下一条。
async fn finished_job(
    bridge: &Bridge,
    room: &Arc<ChatRoom>,
    state: &mut RoomState,
    finished: PromptFinished,
    finished_tx: &mpsc::Sender<PromptFinished>,
) -> BridgeResult<()> {
    state.active = false;
    state.last_activity = Instant::now();
    let lang = bridge.lang(&state.owner).await;
    report_prompt(room, finished.result, lang).await;
    if let Some(job) = state.pending_interrupt.take() {
        start_or_report(bridge, room, state, job, finished_tx, lang).await?;
    } else if let Some(job) = state.queue.pop_front() {
        start_or_report(bridge, room, state, job, finished_tx, lang).await?;
    }
    Ok(())
}

/// 空闲太久：关掉 Agent 会话，上下文留到下次用时由它自己恢复。
async fn close_idle_session(
    bridge: &Bridge,
    room: &ChatRoom,
    state: &mut RoomState,
) -> BridgeResult<()> {
    let Some(session_id) = &state.session_id else {
        return Ok(());
    };
    match bridge.acp.close(session_id).await {
        Ok(()) => state.loaded = false,
        Err(error) => {
            let lang = bridge.lang(&state.owner).await;
            room.send_text(&format_error(&error, lang)).await?;
        }
    }
    state.last_activity = Instant::now();
    Ok(())
}

async fn route_owner_messages(
    bridge: &Bridge,
    room: &Arc<ChatRoom>,
    state: &mut RoomState,
    login: &LoginService,
    messages: &[Message],
    seen: &mut HashSet<String>,
    finished_tx: &mpsc::Sender<PromptFinished>,
) -> BridgeResult<()> {
    // 群里别的 Bot 也能收到这条消息，命令要提及本 Bot 才算数；私聊不用。
    let group = is_group(&bridge.matrix, room, &bridge.config.owner).await;
    for incoming in new_owner_messages(messages, &bridge.config.owner, &bridge.bot, group, seen) {
        receive_job(bridge, room, state, login, group, incoming, finished_tx).await?;
    }
    Ok(())
}

async fn handle_notice(
    commands: &mut Commands,
    room: &ChatRoom,
    bot: &UserId,
    current_session: Option<&str>,
    notice: StdResult<AgentNotice, broadcast::error::RecvError>,
    lang: Lang,
) {
    match notice {
        Ok(AgentNotice::Update { session_id, update }) => {
            commands
                .receive(
                    &RoomSink { room, bot },
                    current_session,
                    &session_id,
                    &update,
                    lang,
                )
                .await;
        }
        Ok(_) => {}
        Err(broadcast::error::RecvError::Lagged(skipped)) => {
            tracing::warn!(skipped, "ACP 通知跟不上，命令菜单可能停在旧表");
        }
        Err(broadcast::error::RecvError::Closed) => {}
    }
}

async fn receive_job(
    bridge: &Bridge,
    room: &Arc<ChatRoom>,
    state: &mut RoomState,
    login: &LoginService,
    group: bool,
    incoming: IncomingText,
    finished_tx: &mpsc::Sender<PromptFinished>,
) -> BridgeResult<()> {
    let lang = bridge.lang(&state.owner).await;
    if incoming.text.is_empty() {
        room.send_text(&text(lang, "input-unsupported", &[]))
            .await?;
        return Ok(());
    }
    if commands::is_help_request(&incoming.text) {
        room.send_text(&commands::help(lang)).await?;
        return Ok(());
    }
    if login
        .receive(
            room,
            group,
            &incoming,
            &bridge.acp,
            &bridge.config.socket,
            state.active,
            lang,
        )
        .await?
    {
        return Ok(());
    }
    let job = match parse_job(&incoming.text, &bridge.config.limits) {
        Ok(job) => job,
        Err(error) => {
            room.send_text(&parse_error(lang, error)).await?;
            return Ok(());
        }
    };
    match job.kind {
        JobKind::Interrupt if state.active => {
            if state.pending_interrupt.is_some() {
                room.send_text(&text(lang, "interrupt-pending", &[]))
                    .await?;
            } else {
                state.pending_interrupt = Some(job);
                let Some(session_id) = state.session_id.as_deref() else {
                    state.pending_interrupt = None;
                    room.send_text(&text(lang, "no-active-session", &[]))
                        .await?;
                    return Ok(());
                };
                if let Err(error) = bridge.acp.cancel(session_id).await {
                    state.pending_interrupt = None;
                    room.send_text(&format_error(&error, lang)).await?;
                }
            }
        }
        JobKind::Steer if state.active && bridge.acp.capabilities().steering => {
            let Some(session_id) = state.session_id.as_deref() else {
                room.send_text(&text(lang, "no-active-session", &[]))
                    .await?;
                return Ok(());
            };
            let outcome = match bridge.acp.steer(session_id, &job.text).await {
                Ok(outcome) => outcome,
                Err(error) => {
                    room.send_text(&format_error(&error, lang)).await?;
                    return Ok(());
                }
            };
            match outcome.as_str() {
                "injected" => {}
                "promptRequired" => {
                    if state.queue.len() >= bridge.config.limits.max_queue {
                        room.send_text(&text(lang, "queue-full", &[])).await?;
                    } else {
                        state.queue.push_back(Job {
                            kind: JobKind::Normal,
                            ..job
                        });
                    }
                }
                _ => room.send_text(&text(lang, "steer-rejected", &[])).await?,
            }
        }
        _ if state.active => {
            if state.queue.len() >= bridge.config.limits.max_queue {
                room.send_text(&text(lang, "queue-full", &[])).await?;
            } else {
                state.queue.push_back(job);
            }
        }
        _ => start_or_report(bridge, room, state, job, finished_tx, lang).await?,
    }
    Ok(())
}

async fn start_job(
    bridge: &Bridge,
    room: &Arc<ChatRoom>,
    state: &mut RoomState,
    job: Job,
    finished_tx: &mpsc::Sender<PromptFinished>,
    lang: Lang,
) -> BridgeResult<()> {
    let (session_id, context_lost) = ensure_session(bridge, room, state).await?;
    if context_lost {
        room.send_text(&text(lang, "session-lost-new", &[])).await?;
    }
    state.active = true;
    state.last_activity = Instant::now();
    let acp = bridge.acp.clone();
    let room = room.clone();
    let limits = bridge.config.limits.clone();
    let tx = finished_tx.clone();
    tokio::spawn(async move {
        let _ = room.typing_notice(true).await;
        let result = acp
            .prompt(
                &session_id,
                &job.text,
                limits.max_output_bytes,
                limits.turn_timeout,
                lang,
            )
            .await;
        let _ = room.typing_notice(false).await;
        let _ = tx.send(PromptFinished { result }).await;
    });
    Ok(())
}

async fn start_or_report(
    bridge: &Bridge,
    room: &Arc<ChatRoom>,
    state: &mut RoomState,
    job: Job,
    finished_tx: &mpsc::Sender<PromptFinished>,
    lang: Lang,
) -> BridgeResult<()> {
    if let Err(error) = start_job(bridge, room, state, job, finished_tx, lang).await {
        room.send_text(&format_error(&error, lang)).await?;
    }
    Ok(())
}

async fn ensure_session(
    bridge: &Bridge,
    room: &Arc<ChatRoom>,
    state: &mut RoomState,
) -> BridgeResult<(String, bool)> {
    if state.session_id.is_none()
        && let Some(record) = bridge.sessions.get(room.room_id().as_str()).await
    {
        state.session_id = Some(record.session_id);
    }
    let mut lost = false;
    if let Some(session_id) = &state.session_id {
        if state.loaded {
            return Ok((session_id.clone(), false));
        }
        let record = SessionRecord {
            session_id: session_id.clone(),
            cwd: bridge.config.workspace.clone(),
        };
        match bridge
            .acp
            .load_or_resume(&record.session_id, &record.cwd, &bridge.config.mcp_url)
            .await
        {
            Ok(()) => {
                state.loaded = true;
                return Ok((record.session_id, false));
            }
            Err(Error::SessionUnavailable(_)) => {
                lost = true;
                bridge.sessions.remove(room.room_id().as_str()).await?;
                state.session_id = None;
            }
            Err(error) => return Err(error),
        }
    }
    let session_id = bridge
        .acp
        .new_session(
            &bridge.config.workspace,
            &bridge.config.mcp_url,
            &bridge.config.mode,
        )
        .await?;
    bridge
        .sessions
        .set(
            room.room_id().to_string(),
            SessionRecord {
                session_id: session_id.clone(),
                cwd: bridge.config.workspace.clone(),
            },
        )
        .await?;
    state.session_id = Some(session_id.clone());
    state.loaded = true;
    Ok((session_id, lost))
}

async fn report_prompt(room: &ChatRoom, result: BridgeResult<PromptResult>, lang: Lang) {
    match result {
        Ok(result) => {
            if let Some(detail) = result.permission_error {
                let message = detail.format(lang);
                let _ = room
                    .send_text(&text(
                        lang,
                        "permission-failed",
                        &[("message", wonton_l10n::Arg::from(message))],
                    ))
                    .await;
            } else if result.permissions_cancelled > 0 {
                let _ = room
                    .send_text(&text(lang, "permissions-cancelled", &[]))
                    .await;
            }
            if result.output_limited {
                let _ = room.send_text(&text(lang, "output-limited", &[])).await;
            } else if result.stop_reason != "cancelled" && result.text.trim().is_empty() {
                let _ = room
                    .send_text(&text(
                        lang,
                        "no-text",
                        &[(
                            "stop_reason",
                            wonton_l10n::Arg::from(result.stop_reason.as_str()),
                        )],
                    ))
                    .await;
            } else if result.stop_reason != "cancelled" {
                let _ = room.send_text(&result.text).await;
            }
        }
        Err(error) => {
            let _ = room.send_text(&format_error(&error, lang)).await;
        }
    }
}

async fn stop_for_agent_disconnect(
    room: &ChatRoom,
    commands: &mut Commands,
    bot: &UserId,
    lang: Lang,
) -> BridgeResult<()> {
    commands.reset(&RoomSink { room, bot }, lang).await;
    let _ = room
        .send_text(&text(lang, "transport-disconnected", &[]))
        .await;
    Err(Error::TransportUnavailable(Detail::msg(
        "acp-connection-closed",
    )))
}
