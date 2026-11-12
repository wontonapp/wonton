use std::{
    collections::{HashSet, VecDeque},
    result::Result as StdResult,
    sync::Arc,
    time::Instant,
};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use tokio::sync::{broadcast, mpsc};
use wonton_matrix::{ChatRoom, MatrixSession, Message, SessionConfig, StoreConfig, UserId};

use crate::{
    acp::{AcpClient, AgentNotice, PromptResult},
    commands::{self, CommandSink, Commands},
    config::{Config, Limits},
    error::{Error, Result as BridgeResult},
    login::{LoginFlow, LoginSink},
    matrix::{IncomingText, new_owner_messages},
    state::{SessionRecord, SessionStore},
};

#[derive(Clone)]
struct Bridge {
    config: Config,
    matrix: Arc<MatrixSession>,
    acp: Arc<AcpClient>,
    sessions: Arc<SessionStore>,
}

#[derive(Clone, Debug)]
enum JobKind {
    Normal,
    Queue,
    Interrupt,
    Steer,
}

#[derive(Clone, Debug)]
struct Job {
    text: String,
    kind: JobKind,
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

struct RoomState {
    session_id: Option<String>,
    loaded: bool,
    active: bool,
    queue: VecDeque<Job>,
    pending_interrupt: Option<Job>,
    last_activity: Instant,
    login: LoginFlow,
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
    let room = Arc::new(
        matrix
            .watch_room(&config.home_room)
            .await
            .context("打开 Matrix 主房间")?,
    );
    // 连上 Agent 前先摆出桥自己的命令，Agent 可能一直不报命令表。
    let bot = matrix.user_id().to_owned();
    let mut commands = Commands::new();
    commands
        .reset(&RoomSink {
            room: &room,
            bot: &bot,
        })
        .await;
    let acp = connect_agent(&config).await?;
    let sessions = Arc::new(SessionStore::open(config.bridge_data.join("sessions.json")).await?);
    let bridge = Bridge {
        config,
        matrix: Arc::new(matrix),
        acp: Arc::new(acp),
        sessions,
    };
    run_room(bridge, room, commands)
        .await
        .map_err(|error| anyhow::anyhow!(error))
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
        .unwrap_or_else(|_| Err(Error::AgentNotReady("ACP initialize 超时".into())));
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

async fn run_room(bridge: Bridge, room: Arc<ChatRoom>, mut commands: Commands) -> BridgeResult<()> {
    let bot = bridge.matrix.user_id().to_owned();
    let mut updates = room.message_updates().await;
    let mut acp_notices = bridge.acp.subscribe();
    let mut acp_closed = bridge.acp.closed();
    if *acp_closed.borrow() {
        return stop_for_agent_disconnect(&room, &mut commands, &bot).await;
    }
    let initial: Vec<Message> = updates.next().await.unwrap_or_default();
    let mut seen = initial
        .iter()
        .filter_map(|message| message.event_id.as_ref().map(ToString::to_string))
        .collect::<HashSet<_>>();
    let (finished_tx, mut finished_rx) = mpsc::channel(16);
    let (login, mut login_rx) = LoginFlow::new();
    let mut state = RoomState {
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
        login,
    };
    let mut idle_tick = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        tokio::select! {
            Some(messages) = updates.next() => {
                route_owner_messages(
                    &bridge,
                    &room,
                    &mut state,
                    &bot,
                    &messages,
                    &mut seen,
                    &finished_tx,
                )
                .await?;
            }
            notice = acp_notices.recv() => {
                handle_notice(&mut commands, &room, &bot, state.session_id.as_deref(), notice)
                    .await;
            }
            Some(finished) = finished_rx.recv() => {
                state.active = false;
                state.last_activity = Instant::now();
                report_prompt(&room, finished.result).await;
                if let Some(job) = state.pending_interrupt.take() {
                    start_or_report(&bridge, &room, &mut state, job, &finished_tx).await?;
                } else if let Some(job) = state.queue.pop_front() {
                    start_or_report(&bridge, &room, &mut state, job, &finished_tx).await?;
                }
            }
            Some(event) = login_rx.recv() => {
                if state
                    .login
                    .event(event, &RoomSink { room: room.as_ref(), bot: &bot })
                    .await?
                {
                    // 终端登录成功后重开 ACP 连接：进程退出码 0，Compose 拉起后重新 initialize。
                    room.send_text("Agent 登录完成，桥将重连。").await?;
                    return Ok(());
                }
            }
            _ = idle_tick.tick() => {
                if !state.active
                    && state.last_activity.elapsed() >= bridge.config.limits.idle_timeout
                    && let Some(session_id) = &state.session_id
                {
                    match bridge.acp.close(session_id).await {
                        Ok(()) => state.loaded = false,
                        Err(error) => room.send_text(&format_error(&error)).await?,
                    }
                    state.last_activity = Instant::now();
                }
            }
            changed = acp_closed.changed() => {
                if changed.is_ok() && *acp_closed.borrow() {
                    return stop_for_agent_disconnect(&room, &mut commands, &bot).await;
                }
            }
            else => return Err(Error::TransportUnavailable("Matrix 房间消息流已关闭".into())),
        }
    }
}

async fn route_owner_messages(
    bridge: &Bridge,
    room: &Arc<ChatRoom>,
    state: &mut RoomState,
    bot: &UserId,
    messages: &[Message],
    seen: &mut HashSet<String>,
    finished_tx: &mpsc::Sender<PromptFinished>,
) -> BridgeResult<()> {
    // 群里别的 Bot 也能收到这条消息，命令要提及本 Bot 才算数；成员数取不到时按私聊处理。
    let group = match room.joined_member_count().await {
        Ok(count) => count > 2,
        Err(error) => {
            tracing::warn!(error = %error, "取不到房间成员数，按私聊处理");
            false
        }
    };
    for incoming in new_owner_messages(messages, &bridge.config.owner, bot, group, seen) {
        receive_job(bridge, room, state, bot, incoming, finished_tx).await?;
    }
    Ok(())
}

async fn handle_notice(
    commands: &mut Commands,
    room: &ChatRoom,
    bot: &UserId,
    current_session: Option<&str>,
    notice: StdResult<AgentNotice, broadcast::error::RecvError>,
) {
    match notice {
        Ok(AgentNotice::Update { session_id, update }) => {
            commands
                .receive(
                    &RoomSink { room, bot },
                    current_session,
                    &session_id,
                    &update,
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
    bot: &UserId,
    incoming: IncomingText,
    finished_tx: &mpsc::Sender<PromptFinished>,
) -> BridgeResult<()> {
    if incoming.text.is_empty() {
        room.send_text("输入不支持：ACP 桥 C2 只接受普通文字。")
            .await?;
        return Ok(());
    }
    if commands::is_help_request(&incoming.text) {
        room.send_text(&commands::help()).await?;
        return Ok(());
    }
    if state
        .login
        .receive(
            &incoming.text,
            &incoming.event_id,
            &RoomSink {
                room: room.as_ref(),
                bot,
            },
            &bridge.acp,
            &bridge.config.socket,
            state.active,
        )
        .await?
    {
        return Ok(());
    }
    let job = match parse_job(&incoming.text, &bridge.config.limits) {
        Ok(job) => job,
        Err(error) => {
            room.send_text(&format_error(&error)).await?;
            return Ok(());
        }
    };
    match job.kind {
        JobKind::Interrupt if state.active => {
            if state.pending_interrupt.is_some() {
                room.send_text("忙：已有一个打断请求在等待处理。").await?;
            } else {
                state.pending_interrupt = Some(job);
                let Some(session_id) = state.session_id.as_deref() else {
                    state.pending_interrupt = None;
                    room.send_text(&format_error(&Error::AgentNotReady(
                        "没有活动 ACP session".into(),
                    )))
                    .await?;
                    return Ok(());
                };
                if let Err(error) = bridge.acp.cancel(session_id).await {
                    state.pending_interrupt = None;
                    room.send_text(&format_error(&error)).await?;
                }
            }
        }
        JobKind::Steer if state.active && bridge.acp.capabilities().steering => {
            let Some(session_id) = state.session_id.as_deref() else {
                room.send_text(&format_error(&Error::AgentNotReady(
                    "没有活动 ACP session".into(),
                )))
                .await?;
                return Ok(());
            };
            let outcome = match bridge.acp.steer(session_id, &job.text).await {
                Ok(outcome) => outcome,
                Err(error) => {
                    room.send_text(&format_error(&error)).await?;
                    return Ok(());
                }
            };
            match outcome.as_str() {
                "injected" => {}
                "promptRequired" => {
                    if state.queue.len() >= bridge.config.limits.max_queue {
                        let error = Error::Busy("ACP 队列已满，请稍后再试。".into());
                        room.send_text(&format_error(&error)).await?;
                    } else {
                        state.queue.push_back(Job {
                            kind: JobKind::Normal,
                            ..job
                        });
                    }
                }
                _ => room.send_text("Agent 没有接受插话请求。").await?,
            }
        }
        _ if state.active => {
            if state.queue.len() >= bridge.config.limits.max_queue {
                let error = Error::Busy("ACP 队列已满，请稍后再试。".into());
                room.send_text(&format_error(&error)).await?;
            } else {
                state.queue.push_back(job);
            }
        }
        _ => start_or_report(bridge, room, state, job, finished_tx).await?,
    }
    Ok(())
}

fn parse_job(text: &str, limits: &Limits) -> BridgeResult<Job> {
    if text.len() > limits.max_input_bytes {
        return Err(Error::InvalidInput(format!(
            "输入超过 {} 字节限制",
            limits.max_input_bytes
        )));
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidInput("输入不能为空".into()));
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
        return Err(Error::InvalidInput("控制命令缺少文字".into()));
    }
    Ok(Job {
        kind,
        text: body.trim().to_owned(),
    })
}

async fn start_job(
    bridge: &Bridge,
    room: &Arc<ChatRoom>,
    state: &mut RoomState,
    job: Job,
    finished_tx: &mpsc::Sender<PromptFinished>,
) -> BridgeResult<()> {
    let (session_id, context_lost) = ensure_session(bridge, room, state).await?;
    if context_lost {
        room.send_text("ACP 会话已丢失，已新建会话；之前的上下文不再可用。")
            .await?;
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
) -> BridgeResult<()> {
    if let Err(error) = start_job(bridge, room, state, job, finished_tx).await {
        room.send_text(&format_error(&error)).await?;
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

async fn report_prompt(room: &ChatRoom, result: BridgeResult<PromptResult>) {
    match result {
        Ok(result) => {
            if let Some(message) = result.permission_error {
                let _ = room
                    .send_text(&format!("Agent 权限请求失败：{message}"))
                    .await;
            } else if result.permissions_cancelled > 0 {
                let _ = room
                    .send_text("权限请求已取消，Agent 没有执行该操作。")
                    .await;
            }
            if result.output_limited {
                let _ = room
                    .send_text("上游错误：Agent 回复超过输出限制，已取消本轮。")
                    .await;
            } else if result.stop_reason != "cancelled" && result.text.trim().is_empty() {
                let _ = room
                    .send_text(&format!(
                        "Agent 本轮没有产出文本（stopReason={}）。",
                        result.stop_reason
                    ))
                    .await;
            } else if result.stop_reason != "cancelled" {
                let _ = room.send_text(&result.text).await;
            }
        }
        Err(error) => {
            let _ = room.send_text(&format_error(&error)).await;
        }
    }
}

async fn stop_for_agent_disconnect(
    room: &ChatRoom,
    commands: &mut Commands,
    bot: &UserId,
) -> BridgeResult<()> {
    commands.reset(&RoomSink { room, bot }).await;
    let _ = room
        .send_text("传输不可用：ACP Agent 连接断开，桥将退出并由容器重启。")
        .await;
    Err(Error::TransportUnavailable("ACP Agent 连接断开".into()))
}

fn format_error(error: &Error) -> String {
    match error {
        Error::AuthenticationRequired => "Agent 需要登录：发送 /wonton login 查看登录方式。".into(),
        Error::AgentNotReady(message) => format!("Agent 未就绪：{message}"),
        Error::TransportUnavailable(message) => format!("传输不可用：{message}"),
        Error::SessionUnavailable(message) => format!("ACP 会话不可用：{message}"),
        Error::Upstream(message) => format!("上游错误：{message}"),
        Error::InvalidInput(message) => format!("输入非法：{message}"),
        Error::ConfirmationUnavailable(message) => format!("确认卡片不可用：{message}"),
        Error::Busy(message) => format!("忙：{message}"),
        Error::Matrix(message) => format!("Matrix 错误：{message}"),
    }
}
