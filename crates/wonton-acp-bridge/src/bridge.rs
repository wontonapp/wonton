//! 桥的主循环：一个房间一个循环，房间里有主对话和各话题的 ACP 会话（ADR-0058）。
//!
//! 消息按所属话题分给会话，每轮开头注入别的会话的动态；话题做完一轮在主对话里留一条
//! `m.notice`；改同一个文件时权限卡片排队。会话状态、工作板和改文件表都在本模块的
//! 子模块里，`run_room` 只做事件分发。
//!
//! 停机期间（容器重建、升级）发来的消息在重启后的首屏快照里：按已读位置（`LastSeen`）
//! 认出来，补处理最近几条，超出的在房间里如实列出（`routing::route_backlog`）。

pub(crate) mod attachments;
mod busy;
mod permissions;
mod refused;
mod routing;
mod run_status;
mod sessions;
mod tasks;
mod turns;
mod updates;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use std::result::Result as StdResult;

use tokio::sync::{broadcast, mpsc};
use wonton_l10n::Lang;
use wonton_matrix::{
    ChatRoom, MatrixSession, Message, OwnedUserId, SessionConfig, StoreConfig, UserId,
};

use crate::{
    acp::{AcpClient, AgentNotice, PermissionAsk, PromptResult},
    commands::{CommandSink, Commands},
    config::Config,
    error::{Detail, Error, Result as BridgeResult, format_error},
    l10n::text,
    locale::{Locales, Origin},
    login::{LoginChat, LoginEvent, LoginFlow, LoginSink},
    room::Room,
    room_owner::room_owner,
    state::{LastSeen, SessionStore},
    topics::{SessionKey, Topics},
    undecryptable,
};

use permissions::ApprovalWaiter;

#[derive(Clone)]
struct Bridge {
    config: Config,
    acp: Arc<AcpClient>,
    sessions: Arc<SessionStore>,
    last_seen: Arc<LastSeen>,
    bot: OwnedUserId,
    locales: Arc<Locales<Origin>>,
}

impl Bridge {
    /// 这个房间说话用的语言（按房间主人，取不到照兜底退）。
    async fn lang(&self, room_owner: &UserId) -> Lang {
        self.locales.lang(room_owner).await
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JobKind {
    Normal,
    /// `/wonton interrupt <文字>`：调试用，不进帮助和菜单。
    Interrupt,
    End,
}

#[derive(Clone, Debug)]
struct Job {
    /// 文字部分：文字消息是正文，媒体消息是说明（caption），都没有时为空。
    text: String,
    kind: JobKind,
    /// 这条请求的消息事件 ID，状态消息的 `run`（ADR-0082）。
    event_id: String,
    /// 已落工作区的附件；纯文字为 None。
    media: Option<attachments::PreparedAttachment>,
}

impl Job {
    /// 给工作板和话题开局看的用户输入摘要：附件没有说明时用文件名。
    fn display(&self) -> &str {
        match (self.text.trim(), &self.media) {
            ("", Some(media)) => &media.name,
            _ => &self.text,
        }
    }
}

struct PromptFinished {
    key: SessionKey,
    result: BridgeResult<PromptResult>,
}

/// 一轮里要用的东西：房间、发送目标、房间是不是群、说话的语言。会话状态和工作板另外按可变引用传。
struct TurnContext<R: Room> {
    bridge: Bridge,
    room: Arc<R>,
    login: Arc<LoginService<R>>,
    group: bool,
    lang: Lang,
    finished_tx: mpsc::Sender<PromptFinished>,
    ask_tx: mpsc::Sender<PermissionAsk>,
    cards_tx: mpsc::Sender<permissions::CardDone>,
}

struct RoomSink<'a, R: Room> {
    room: &'a R,
    bot: &'a UserId,
    /// 正在话题里时，回复也发进这个话题。
    thread: Option<&'a str>,
}

impl<R: Room> RoomSink<'_, R> {
    async fn text(&self, text: &str) -> BridgeResult<()> {
        match self.thread {
            Some(root) => self.room.send_thread_text(root, text).await,
            None => self.room.send_text(text).await,
        }
    }
}

impl<R: Room> CommandSink for RoomSink<'_, R> {
    async fn set_commands(&self, content: &serde_json::Value) -> wonton_matrix::Result<()> {
        self.room
            .set_state(
                wonton_protocol::BOT_COMMANDS_EVENT,
                self.bot.as_str(),
                content.clone(),
            )
            .await
            .map_err(|error| wonton_matrix::Error::Upstream(error.to_string()))
    }

    async fn send_command_text(&self, text: &str) -> wonton_matrix::Result<()> {
        self.text(text)
            .await
            .map_err(|error| wonton_matrix::Error::Upstream(error.to_string()))
    }
}

impl<R: Room> LoginSink for RoomSink<'_, R> {
    async fn send_login_text(&self, text: &str) -> BridgeResult<()> {
        self.text(text).await
    }
    async fn redact(&self, event_id: &str, reason: &str) -> BridgeResult<()> {
        self.room.redact(event_id, reason).await
    }
}

/// 登录是全 Agent 一份的：群里发起的命令转到和主人的私聊，事件也只在私聊里回。
struct LoginService<R: Room> {
    flow: tokio::sync::Mutex<LoginFlow>,
    /// 主人和 Bot 的私聊；主房间本身是私聊时为 None。
    direct: Option<Arc<R>>,
    bot: wonton_matrix::OwnedUserId,
    acp: Arc<AcpClient>,
    socket: std::path::PathBuf,
}

impl<R: Room> LoginService<R> {
    // 房间、群标记、消息、ACP、socket、忙状态、语言互相独立，再包一层也是逐个填。
    #[allow(clippy::too_many_arguments)]
    async fn receive(
        &self,
        room: &Arc<R>,
        group: bool,
        thread: Option<&str>,
        text: &str,
        event_id: &str,
        busy: bool,
        lang: Lang,
    ) -> BridgeResult<bool> {
        let bot = &self.bot;
        let current = RoomSink { room, bot, thread };
        let direct = self.direct.as_ref().map(|room| RoomSink {
            room,
            bot,
            thread: None,
        });
        self.flow
            .lock()
            .await
            .receive_from(
                text,
                event_id,
                &LoginChat {
                    current: &current,
                    direct: direct.as_ref(),
                    group,
                },
                &self.acp,
                &self.socket,
                busy,
                lang,
            )
            .await
    }

    async fn event(&self, event: LoginEvent, room: &R, lang: Lang) -> BridgeResult<bool> {
        self.flow
            .lock()
            .await
            .event(
                event,
                &RoomSink {
                    room,
                    bot: &self.bot,
                    thread: None,
                },
                lang,
            )
            .await
    }
}

struct RoomState {
    owner: OwnedUserId,
    sessions: sessions::Sessions,
    /// 一轮一条的 Agent 进度状态消息（ADR-0082）。
    run_status: run_status::Board,
    /// 等对方做完再改的权限请求。
    waiters: Vec<ApprovalWaiter>,
    /// 主对话最近的文字消息（事件 ID 和原文），拼话题开局时取话题起点和前文。
    history: VecDeque<(String, String)>,
    last_activity: Instant,
    /// 被网关拒收的自己的消息。
    refused: refused::Refused,
    /// 上次清理工作区附件的时间。
    last_sweep: Instant,
    /// 解不开的加密消息（`undecryptable`）。
    undecryptable: undecryptable::Tracker,
}

/// 主对话里存多少条最近消息；开局最多带 6 条前文，留够余量。
const HISTORY_MAX: usize = 20;

/// 断线收尾最多等多久：在跑的轮次会马上以传输错误结束，等它们交回结果（ADR-0082）。
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Agent 断了：在跑的轮次马上会以传输错误结束，等它们交回结果（错误照发、状态消息
/// 按 ADR-0082 收尾），不再开新轮；收完退出由 Compose 重启、启动恢复处理遗留状态。
async fn drain_in_flight<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    finished_rx: &mut mpsc::Receiver<PromptFinished>,
) -> BridgeResult<()> {
    state.sessions.drop_pending();
    let deadline = Instant::now() + DRAIN_GRACE;
    while state.run_status.busy() {
        let Some(finished) =
            tokio::time::timeout_at(tokio::time::Instant::from(deadline), finished_rx.recv())
                .await
                .ok()
                .flatten()
        else {
            return Ok(());
        };
        turns::finished(context, state, topics, finished).await?;
    }
    Ok(())
}

/// 打开一轮一条的状态消息看板，把重启前还停在 working/waiting 的编辑成不确定。
async fn open_run_status<R: Room>(bridge: &Bridge, room: &R) -> run_status::Board {
    let mut board =
        run_status::Board::open(bridge.config.bridge_data.clone(), room.room_id().as_str()).await;
    board.recover(room).await;
    board
}

/// 一轮循环的发送上下文：三对通道在这里建好，接收端交回循环，发送端装进 [`TurnContext`]。
#[allow(clippy::type_complexity)]
fn turn_context<R: Room>(
    bridge: &Bridge,
    room: Arc<R>,
    login: Arc<LoginService<R>>,
    group: bool,
    lang: Lang,
) -> (
    TurnContext<R>,
    mpsc::Receiver<PromptFinished>,
    mpsc::Receiver<PermissionAsk>,
    mpsc::Receiver<permissions::CardDone>,
) {
    let (finished_tx, finished_rx) = mpsc::channel(16);
    let (ask_tx, ask_rx) = mpsc::channel(8);
    let (cards_tx, cards_rx) = mpsc::channel(8);
    (
        TurnContext {
            bridge: bridge.clone(),
            room,
            login,
            group,
            lang,
            finished_tx,
            ask_tx,
            cards_tx,
        },
        finished_rx,
        ask_rx,
        cards_rx,
    )
}

pub async fn run() -> Result<()> {
    let config = Config::from_env()?;
    tokio::fs::create_dir_all(&config.bridge_data)
        .await
        .context("创建桥数据目录")?;
    // 工作区里上次的附件按保留期和总量清一轮；失败只记日志。
    attachments::sweep(&config.workspace).await;
    let health = config.health.clone();
    health.touch_alive();
    // 心跳让容器的健康检查知道主进程活着；进程退出文件自然就不新了。
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            tick.tick().await;
            health.touch_alive();
        }
    });
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
    let group = is_group(&matrix, &room, &config.owner).await;
    let direct = if group {
        watch_direct(&matrix, &config.owner).await
    } else {
        None
    };
    let (login, login_rx) = LoginFlow::new();
    let acp = Arc::new(connect_agent(&config).await?);
    let login = Arc::new(LoginService {
        flow: tokio::sync::Mutex::new(login),
        direct: direct.clone(),
        bot: bot.clone(),
        acp: acp.clone(),
        socket: config.socket.clone(),
    });
    let sessions = Arc::new(SessionStore::open(config.bridge_data.join("sessions.json")).await?);
    let last_seen = Arc::new(LastSeen::open(config.bridge_data.join("last_seen.json")).await?);
    let locales = Arc::new(Locales::new(
        Origin::core(&config.mcp_url),
        config.owner.clone(),
    ));
    let bridge = Bridge {
        config,
        acp,
        sessions,
        last_seen,
        bot,
        locales,
    };
    match direct {
        Some(direct) => {
            // 登录事件只由私聊循环消费；它结束后整座桥退出重连。
            let mut direct_loop = tokio::spawn(run_room(
                bridge.clone(),
                direct,
                group,
                Some(login_rx),
                login.clone(),
            ));
            let result = tokio::select! {
                result = run_room(bridge, room, group, None, login) => result,
                result = &mut direct_loop => match result {
                    Ok(result) => result,
                    Err(error) => Err(join_error(error)),
                },
            };
            direct_loop.abort();
            result.map_err(|error| anyhow::anyhow!(error))
        }
        None => run_room(bridge, room, group, Some(login_rx), login)
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

/// 循环起跑前的准备：认主人、按主人的语言摆桥自己的命令（Agent 可能一直不报命令表）。
/// 返回主人和这个房间说话用的语言。
async fn prepare_room<R: Room>(
    bridge: &Bridge,
    room: &R,
    bot: &UserId,
    commands: &mut Commands,
) -> (OwnedUserId, Lang) {
    let owner = room_owner(room, bot, &bridge.config.owner).await;
    let lang = bridge.lang(&owner).await;
    commands
        .reset(
            &RoomSink {
                room,
                bot,
                thread: None,
            },
            lang,
        )
        .await;
    (owner, lang)
}

/// 起跑时的新桥内状态：会话、已读、解不开消息的记录都从零开始；运行状态按房间开。
async fn fresh_state<R: Room>(bridge: &Bridge, room: &R, owner: OwnedUserId) -> RoomState {
    RoomState {
        owner,
        sessions: sessions::Sessions::default(),
        run_status: open_run_status(bridge, room).await,
        waiters: Vec::new(),
        history: VecDeque::new(),
        last_activity: Instant::now(),
        refused: refused::Refused::default(),
        last_sweep: Instant::now(),
        undecryptable: undecryptable::Tracker::default(),
    }
}

async fn run_room<R: Room>(
    bridge: Bridge,
    room: Arc<R>,
    group: bool,
    mut login_events: Option<mpsc::Receiver<LoginEvent>>,
    login: Arc<LoginService<R>>,
) -> BridgeResult<()> {
    let bot = bridge.bot.clone();
    let mut commands = Commands::new();
    let (owner, start_lang) = prepare_room(&bridge, room.as_ref(), &bot, &mut commands).await;
    let mut topics = open_topics(bridge.config.bridge_data.join("topics"), room.as_ref()).await?;
    let mut updates = room.updates().await;
    let mut acp_notices = bridge.acp.subscribe();
    let mut acp_closed = bridge.acp.closed();
    if *acp_closed.borrow() {
        return stop_for_agent_disconnect(room.as_ref(), &mut commands, &bot, start_lang).await;
    }
    let initial: Vec<Message> = updates.next().await.unwrap_or_default();
    let mut seen = HashSet::new();
    // 本轮到过的事件（含没解开的）：迟解密重投、重启竞态下旧消息当增量到达，都靠它分辨。
    let mut arrived = HashSet::new();
    let (context, mut finished_rx, mut ask_rx, mut cards_rx) =
        turn_context(&bridge, room.clone(), login, group, start_lang);
    let mut state = fresh_state(&bridge, room.as_ref(), owner).await;
    // 进循环前先分发首屏快照：补上停机期间发来的，超出上限的如实列出。
    routing::route_backlog(
        &context,
        &mut state,
        &mut topics,
        &initial,
        &mut seen,
        &mut arrived,
    )
    .await?;
    let mut idle_tick = tokio::time::interval(bridge.config.limits.idle_tick);
    // 步骤变化的合并编辑每 3 秒最多一次；每秒看一眼有没有攒着的要发。
    let mut status_tick = tokio::time::interval(std::time::Duration::from_secs(1));
    // 扫“解不开”的节奏：时限的 1/4，最快 100 毫秒扫一次（测试把时限配短）。
    let mut undecryptable_tick = tokio::time::interval(
        (bridge.config.limits.undecryptable_after / 4).max(std::time::Duration::from_millis(100)),
    );
    loop {
        tokio::select! {
            Some(messages) = updates.next() => {
                state.undecryptable.observe(&messages, &bridge.config.allowed_users);
                on_messages(
                    &context,
                    &mut state,
                    &mut topics,
                    &messages,
                    &mut seen,
                    &mut arrived,
                )
                .await?;
            }
            notice = acp_notices.recv() => {
                handle_acp_notice(&context, &mut state, &mut topics, &mut commands, notice).await?;
            }
            Some(finished) = finished_rx.recv() => {
                turns::finished(&context, &mut state, &mut topics, finished).await?;
            }
            Some(ask) = ask_rx.recv() => {
                permissions::handle(&context, &mut state, &mut topics, ask).await?;
            }
            Some(done) = cards_rx.recv() => {
                permissions::settle(&context, &mut state, &mut topics, done).await?;
            }
            Some(event) = next_login_event(&mut login_events) => {
                let lang = bridge.lang(&state.owner).await;
                if context.login.event(event, room.as_ref(), lang).await? {
                    // 终端登录成功后重开 ACP 连接：进程退出码 0，Compose 拉起后重新 initialize。
                    room.send_text(&text(lang, "login-reconnect", &[])).await?;
                    return Ok(());
                }
            }
            _ = idle_tick.tick() => {
                turns::idle(&context, &mut state, &mut topics).await?;
            }
            _ = status_tick.tick() => {
                state.run_status.flush(room.as_ref(), Instant::now()).await;
            }
            _ = undecryptable_tick.tick() => {
                report_undecryptable(&bridge, &mut state, room.as_ref()).await;
            }
            changed = acp_closed.changed() => {
                if changed.is_ok() && *acp_closed.borrow() {
                    drain_in_flight(&context, &mut state, &mut topics, &mut finished_rx).await?;
                    return stop_for_agent_disconnect(
                        room.as_ref(),
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

/// 房间时间线变了：先看自己被拒收的消息，再把人的新消息交给会话。
async fn on_messages<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    messages: &[Message],
    seen: &mut HashSet<String>,
    arrived: &mut HashSet<String>,
) -> BridgeResult<()> {
    refused::handle(context, &mut state.refused, messages, seen, &state.owner).await?;
    routing::route_owner_messages(context, state, topics, messages, seen, arrived).await
}

/// 一条 ACP 通知：任务事件开话题，其余交给命令菜单和改文件表。
async fn handle_acp_notice<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    commands: &mut Commands,
    notice: StdResult<AgentNotice, broadcast::error::RecvError>,
) -> BridgeResult<()> {
    match &notice {
        Ok(AgentNotice::Task { params }) => tasks::handle(context, state, topics, params).await,
        _ => updates::handle_notice(context, state, topics, commands, notice).await,
    }
}

/// 解不开的加密消息到点还没解开：在这个房间说一次，并给健康检查写上原因。发失败只记日志，
/// 下一个提醒间隔（`undecryptable_every`）再试。
async fn report_undecryptable<R: Room>(bridge: &Bridge, state: &mut RoomState, room: &R) {
    if !state.undecryptable.report_due(
        bridge.config.limits.undecryptable_after,
        bridge.config.limits.undecryptable_every,
        Instant::now(),
    ) {
        return;
    }
    let lang = bridge.lang(&state.owner).await;
    let notice = text(lang, "crypto-undecryptable", &[]);
    bridge.config.health.mark_degraded(&notice);
    if let Err(error) = room.send_text(&notice).await {
        tracing::warn!(error = %error, "发“解不开”提示失败");
    }
}

async fn open_topics(path: std::path::PathBuf, room: &impl Room) -> BridgeResult<Topics> {
    let file = path.join(format!("{}.json", room.room_id()));
    Topics::open(file, std::time::SystemTime::now()).await
}

async fn stop_for_agent_disconnect<R: Room>(
    room: &R,
    commands: &mut Commands,
    bot: &UserId,
    lang: Lang,
) -> BridgeResult<()> {
    commands
        .reset(
            &RoomSink {
                room,
                bot,
                thread: None,
            },
            lang,
        )
        .await;
    let _ = room
        .send_text(&text(lang, "transport-disconnected", &[]))
        .await;
    Err(Error::TransportUnavailable(Detail::msg(
        "acp-connection-closed",
    )))
}

fn now() -> std::time::SystemTime {
    std::time::SystemTime::now()
}
