//! Agent 调用链账本（ADR-0061 第 10 节，第 2 块）：一句话引出的所有 Agent 会话是一条链，
//! 链上记身份、层数和轮数，随时能停整条链。纯逻辑，不接 HTTP、Bot 插件和 MCP 工具
//! （第 3、6、7 块经这里的公开方法接）；发起人权限、房间表判断（#248）也归它们。
//! 线上类别（[`ChainError`] 等）用 wonton-protocol 已有的；账本自己的类型
//! （会话信息、停链结果、上限事件）只在核心内部第 2/3/6/8 块之间用，不上线。

mod budget;
mod layers;
pub mod settings;
mod stop;
mod store;
#[cfg(test)]
mod tests;
mod turns;
mod wake;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use wonton_protocol::{
    AgentChainBot, AgentChainRow, AgentChainStatus, AgentChainsViewModel, BotRef, ChainError,
    ChainLimitKind, ChainLimits, ChainTurnGrant, ChainTurnReport, ConfirmationOrigin,
};

/// 账本里一个会话的登记：会话 ID → 链、Bot、层。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainSessionInfo {
    pub session: String,
    pub chain: String,
    pub bot: BotRef,
    pub layer: u32,
    /// 这一轮说话的人。
    pub sender: Option<String>,
    /// 还活着：没报结束、没被停链取消。重启后一律为假。
    pub live: bool,
}

/// 这个 Bot 当前进行中的轮次（Go 联邦线 #353 用）：一条链上还活着的一轮和说话人。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveTurn {
    pub chain: String,
    pub session: String,
    pub sender: String,
}

/// 停链的交出项：要取消的活轮次，加上带这条链、还没答的卡片（要撤回）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChainStopOutcome {
    pub turns: Vec<ChainSessionInfo>,
    pub cards: Vec<String>,
}

/// 链到上限的那一下（轮数）。每次进入 `limit_reached` 恰好发出一次；
/// 层数不算——层数超了只拒那一跳，不翻链的状态，也没有"再给一份"。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainLimitEvent {
    pub chain: String,
    pub kind: ChainLimitKind,
}

/// 账本操作的错误。网关要回给 Bot 插件的拒绝包在 [`ChainLedgerError::Rejected`]（线上类别
/// 原样透传）；其余是账本自己的状态错误和落盘失败。
#[derive(Debug, thiserror::Error)]
pub enum ChainLedgerError {
    #[error("链拒绝：{0:?}")]
    Rejected(ChainError),
    #[error("链 {0} 不存在")]
    UnknownChain(String),
    #[error("卡片 {0} 没有登记在账本里")]
    UnknownCard(String),
    #[error("链 {chain} 不在 limit_reached，给不了新的一份")]
    NotAtLimit { chain: String },
    #[error("{0}")]
    Invalid(String),
    #[error("链账本损坏：{path}: {message}")]
    Corrupt { path: PathBuf, message: String },
    #[error("链账本读写失败：{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("链账本序列化失败：{0}")]
    Serialize(String),
}

type LimitHandler = Arc<dyn Fn(&ChainLimitEvent) + Send + Sync>;

#[derive(Clone)]
struct SessionEntry {
    chain: String,
    bot: BotRef,
    layer: u32,
    /// 这一轮说话的人。
    sender: Option<String>,
    live: bool,
}

#[derive(Clone)]
struct ChainCard {
    chain: String,
    answered: bool,
}

#[derive(Clone)]
struct LedgerState {
    /// 新开的链抄一份开链当时的全局上限；设置后来改了只影响新链（旧链用"再给一份"）。
    default_limits: ChainLimits,
    chains: BTreeMap<String, store::StoredChain>,
    sessions: HashMap<String, SessionEntry>,
    /// 消息事件 ID → 链 ID，见 `wake`。内存态，不落盘。
    wake: HashMap<String, String>,
    /// 卡片 ID → 所属链。内存态：重启后链都转 `Ended`，停链撤卡不再发生。
    cards: HashMap<String, ChainCard>,
    /// 每条链最近一次活动的序号（视图排序用），加全局计数器。内存态。
    activity: HashMap<String, u64>,
    tick: u64,
    /// 这次操作里攒下的"用到上限"事件，提交后统一定时发出（见 `mutate`）。
    events: Vec<ChainLimitEvent>,
    /// 这次操作动了要落盘的数据；内存态操作不置位、不写盘。
    dirty: bool,
}

/// 调用链账本。所有方法线程安全；写操作串行（锁内落盘），事件在锁外逐条发给回调。
pub struct Chains {
    state: Mutex<LedgerState>,
    path: PathBuf,
    on_limit: Mutex<Option<LimitHandler>>,
    /// 列表的版本：任何一次提交都加一，App 的链列表长轮询挂在它上面。
    revision: watch::Sender<u64>,
}

impl Chains {
    /// 打开账本：读回数据目录下的账本文件（如 `core_state_dir().join("agent-chains.json")`）。
    /// 进行中的链按 ADR-0061 第 2 节转成 `Ended`（重启时所有连接都断），计数原样保留给 App；
    /// 停过的链维持 `Stopped`。文件缺失等于没有链，坏文件报错不顶替。
    pub fn open(
        path: impl Into<PathBuf>,
        default_limits: ChainLimits,
    ) -> Result<Self, ChainLedgerError> {
        let path = path.into();
        let mut chains = BTreeMap::new();
        for mut chain in store::read(&path)? {
            if matches!(
                chain.status,
                AgentChainStatus::Running | AgentChainStatus::LimitReached
            ) {
                // 重启时所有连接都断：进行中的链没有能取消的会话，记成做完。
                chain.status = AgentChainStatus::Ended;
                chain.limit_kind = None;
            }
            if chains.insert(chain.chain.clone(), chain).is_some() {
                return Err(ChainLedgerError::Corrupt {
                    path: path.clone(),
                    message: "重复的链 ID".into(),
                });
            }
        }
        Ok(Self {
            state: Mutex::new(LedgerState {
                default_limits,
                chains,
                ..LedgerState::empty()
            }),
            path,
            on_limit: Mutex::new(None),
            revision: watch::channel(0).0,
        })
    }

    /// 注册"用到上限"的回调（第 3 块拿它发上限卡片，在开始对外服务前设置一次）。
    /// 回调在账本锁外逐条调用；不要再调回账本，会死锁。
    pub fn on_limit_event(&self, handler: LimitHandler) {
        *self
            .on_limit
            .lock()
            .expect("agent chain ledger lock poisoned") = Some(handler);
    }

    /// 列表的当前版本（[`Self::view`] 的 `revision`）。
    pub fn revision(&self) -> u64 {
        *self.revision.borrow()
    }

    /// 版本变化的订阅：App 的列表长轮询挂在它上面。
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    /// 改全局上限（设置“Agent”一页）：只影响之后新开的链，旧链用“再给一份”。
    pub fn set_default_limits(&self, limits: ChainLimits) {
        let mut state = self.state.lock().expect("agent chain ledger lock poisoned");
        state.default_limits = limits;
        self.bump_revision();
    }

    /// 这个 Bot 当前进行中的轮次（还没 End、链没停）：联邦线按它算这个 Bot 此刻在跟谁说话。
    /// 说话人是登记时报的，链代表的人可能更早。
    pub fn live_turns(&self, bot: &BotRef) -> Vec<LiveTurn> {
        let mut turns = self.read(|state| {
            state
                .sessions
                .iter()
                .filter(|(_, entry)| {
                    entry.live
                        && &entry.bot == bot
                        && state.chains.get(&entry.chain).is_some_and(|chain| {
                            !matches!(
                                chain.status,
                                AgentChainStatus::Stopped | AgentChainStatus::Ended
                            )
                        })
                })
                .filter_map(|(session, entry)| {
                    Some(LiveTurn {
                        chain: entry.chain.clone(),
                        session: session.clone(),
                        sender: entry.sender.clone()?,
                    })
                })
                .collect::<Vec<_>>()
        });
        turns.sort_by(|a, b| a.chain.cmp(&b.chain).then(a.session.cmp(&b.session)));
        turns
    }

    /// Bot 插件的一轮开始（`POST /agent/chains` 的 start 报告）：按触发消息的唤醒标签沿用原链，
    /// 沿用不上就开新链（发起的 Bot 记在第 1 层）。拿到 [`ChainTurnGrant`] 才发 prompt，
    /// 轮数在拿到的那一刻记。说话人是不是这个 Bot 听的人归第 3 块核，账本只记。
    /// `mentioner` 是说话人是链上 Bot 时的那个 Bot（第 8 块：被点名的 Bot 用它的层进链）；
    /// `chain_user` 是链代表的人——说话的人，或点名 Bot 的归属人。
    pub fn start_turn(
        &self,
        bot: BotRef,
        report: ChainTurnReport,
        mentioner: Option<BotRef>,
        chain_user: String,
    ) -> Result<ChainTurnGrant, ChainLedgerError> {
        let ChainTurnReport::Start {
            session,
            room,
            thread,
            sender,
            event_id,
        } = report
        else {
            return Err(ChainLedgerError::Invalid(
                "start_turn 只收 start 报告；end 走 end_session".into(),
            ));
        };
        self.mutate(|state| {
            let origin = ConfirmationOrigin {
                room_id: room,
                thread_id: thread,
            };
            match wake::lookup(state, &event_id) {
                Some(chain_id) => turns::continue_round(
                    state,
                    &chain_id,
                    &bot,
                    &session,
                    &sender,
                    mentioner.as_ref(),
                ),
                None => turns::open_chain(
                    state,
                    chain_user,
                    &sender,
                    origin,
                    store::ChainBotEntry { bot, layer: 1 },
                    &session,
                ),
            }
        })
    }

    /// 一轮结束（Bot 插件的 end 报告、网关侧会话完结）。End 对被拒的轮也会报，
    /// 没登记的会话静默忽略。
    ///
    /// 最后一轮结束说明这条链上没有在跑的轮次了：没到上限的链记成 `Ended`，从 App 的列表里下去；
    /// 点名叫它会回到 `Running`（见 `turns`），停下来的链仍能停下来。到上限的留 `LimitReached`
    /// 等卡片回答，“再给一份”靠它找链。
    pub fn end_session(&self, session: &str) {
        let mut state = self.state.lock().expect("agent chain ledger lock poisoned");
        let Some(entry) = state.sessions.get_mut(session) else {
            return;
        };
        if !std::mem::take(&mut entry.live) {
            return;
        }
        let chain_id = entry.chain.clone();
        if state
            .sessions
            .values()
            .any(|other| other.live && other.chain == chain_id)
        {
            return;
        }
        if let Some(chain) = state.chains.get_mut(&chain_id)
            && chain.status == AgentChainStatus::Running
        {
            chain.status = AgentChainStatus::Ended;
        }
        // 列表这一行不再算在跑：App 靠版本变化收起停止钮。
        self.bump_revision();
    }

    /// 停整条链：置 `Stopped`，交出链上活的轮次和待答的卡片，
    /// 之后一切新登记都是 `stopped`。停是幂等的；做完的链也停（用户按下停止钮时最后一轮可能刚报了 End）。
    pub fn stop(&self, chain_id: &str) -> Result<ChainStopOutcome, ChainLedgerError> {
        self.mutate(|state| stop::stop(state, chain_id))
    }

    /// "再给一份"（上限卡片的按钮）：轮数上限加一份默认值，回到 `Running`。
    /// 只对 `limit_reached` 的链有效；层数和绕圈不在此列。
    pub fn grant_another(&self, chain_id: &str) -> Result<(), ChainLedgerError> {
        self.mutate(|state| {
            let chain = state
                .chains
                .get_mut(chain_id)
                .ok_or_else(|| ChainLedgerError::UnknownChain(chain_id.to_owned()))?;
            if chain.status != AgentChainStatus::LimitReached {
                return Err(ChainLedgerError::NotAtLimit {
                    chain: chain_id.to_owned(),
                });
            }
            budget::grant_another(chain);
            state.dirty = true;
            Ok(())
        })
    }

    /// 链现在的状态；没有这条链时是 `None`。绑卡之后回查一眼，不必拼整行。
    pub fn status(&self, chain_id: &str) -> Option<AgentChainStatus> {
        self.read(|state| state.chains.get(chain_id).map(|chain| chain.status))
    }

    /// 登记一张带这条链的卡片（发消息卡片、权限卡片、上限卡片、插件工具卡片），停链时随活会话一起交出撤回。
    pub fn bind_card(&self, chain_id: &str, card_id: &str) -> Result<(), ChainLedgerError> {
        let mut state = self.state.lock().expect("agent chain ledger lock poisoned");
        if !state.chains.contains_key(chain_id) {
            return Err(ChainLedgerError::UnknownChain(chain_id.to_owned()));
        }
        if state.cards.contains_key(card_id) {
            return Err(ChainLedgerError::Invalid(format!("卡片 {card_id} 已登记")));
        }
        state.cards.insert(
            card_id.to_owned(),
            ChainCard {
                chain: chain_id.to_owned(),
                answered: false,
            },
        );
        Ok(())
    }

    /// 卡片有了答案（批了、拒了、过期）：不再是待答，停链不再撤回它。
    pub fn resolve_card(&self, card_id: &str) -> Result<(), ChainLedgerError> {
        let mut state = self.state.lock().expect("agent chain ledger lock poisoned");
        state
            .cards
            .get_mut(card_id)
            .ok_or_else(|| ChainLedgerError::UnknownCard(card_id.to_owned()))?
            .answered = true;
        Ok(())
    }

    /// 登记唤醒标签：这条消息事件属于这条链（第 6/8 块在链上 Bot 的消息落进对话时调）。
    pub fn bind_event(&self, event_id: &str, chain_id: &str) -> Result<(), ChainLedgerError> {
        let mut state = self.state.lock().expect("agent chain ledger lock poisoned");
        wake::bind(&mut state, event_id, chain_id)
    }

    /// 链上 Bot 的这条消息挂回哪条链（第 8 块在 Bot 发消息的转发路上查）：
    /// 按 Bot、房间、话题找它当时活的链。
    pub fn chain_for_wake(&self, bot: &BotRef, room: &str, thread: Option<&str>) -> Option<String> {
        self.read(|state| wake::chain_for_wake(state, bot, room, thread))
    }

    /// 这条链代表的人（最早说话的那个人）：说话人本身是 Bot 账号时，封顶按他的级别算。
    pub fn chain_user(&self, chain_id: &str) -> Option<String> {
        self.read(|state| state.chains.get(chain_id).map(|chain| chain.user.clone()))
    }

    /// 这个 Bot 眼下活着的那一轮在哪条链上：报出的说话人对得上的候选里挑最近开始的那一轮。
    ///
    /// 说话人认不出（远端好友之类）时不按人过滤，任何活轮次都算候选。
    /// 工具调用自己不带链、也没带会话头（插件没认出这一轮）时靠它把卡片认到链上。挑最近一轮不是硬保证：
    /// 同时开着两轮、报的又是同一个人时分不开，卡片可能认到另一条链，停那条链会把它撤回、这一轮要发再说一句。
    /// 没有活着的轮次就不认——猜错比不认更糟，停链会撤回别人的卡片。
    pub fn live_turn_chain(&self, plugin: &str, bot: &str, sender: Option<&str>) -> Option<String> {
        self.read(|state| {
            state
                .sessions
                .values()
                .filter(|entry| entry.live && entry.bot.plugin_id == plugin && entry.bot.bot == bot)
                .filter(|entry| sender.is_none_or(|sender| entry.sender.as_deref() == Some(sender)))
                .max_by_key(|entry| {
                    state
                        .activity
                        .get(&entry.chain)
                        .copied()
                        .unwrap_or_default()
                })
                .map(|entry| entry.chain.clone())
        })
    }

    /// 按会话找链。
    pub fn session_chain(&self, session: &str) -> Option<ChainSessionInfo> {
        self.read(|state| {
            state.sessions.get(session).map(|entry| ChainSessionInfo {
                session: session.to_owned(),
                chain: entry.chain.clone(),
                bot: entry.bot.clone(),
                layer: entry.layer,
                sender: entry.sender.clone(),
                live: entry.live,
            })
        })
    }

    /// App 的链列表：还在跑的（`Running`、到上限等回答的 `LimitReached`）按最近活动排。
    /// 链上最后那轮报了 End 就记成 `Ended`、从这里下去，停了和做完的留给详情（[`Chains::row`]）。
    /// `can_stop` 恒为真——哪些人看得见、能停（链代表和各 Bot 的归属人）由第 3 块按归属过滤。
    pub fn view(&self) -> AgentChainsViewModel {
        let revision = *self.revision.borrow();
        let mut view = self.read(|state| {
            let mut rows: Vec<(u64, AgentChainRow)> = state
                .chains
                .values()
                .filter(|chain| {
                    matches!(
                        chain.status,
                        AgentChainStatus::Running | AgentChainStatus::LimitReached
                    )
                })
                .map(|chain| {
                    let tick = state.activity.get(&chain.chain).copied().unwrap_or(0);
                    (tick, build_row(state, chain))
                })
                .collect();
            rows.sort_by_key(|(tick, _)| std::cmp::Reverse(*tick));
            AgentChainsViewModel {
                chains: rows.into_iter().map(|(_, row)| row).collect(),
                revision,
                action_failure: None,
            }
        });
        // 读者拿到的版本不能比列表旧：读守着一把锁，期间的提交已经生效。
        view.revision = view.revision.max(*self.revision.borrow());
        view
    }

    /// 单条链（任意状态，App 的链详情用）。
    pub fn row(&self, chain_id: &str) -> Option<AgentChainRow> {
        self.read(|state| {
            state
                .chains
                .get(chain_id)
                .map(|chain| build_row(state, chain))
        })
    }

    /// 落盘类操作：克隆状态 → 改 → 有落盘改动就原子写盘 → 写成功才提交到内存。
    /// 写失败时内存不动（拒绝和改动都没发生），错误原样返回。
    /// 攒下的上限事件在锁释放后逐条发给回调，回调里不能再进账本。
    fn mutate<T>(
        &self,
        f: impl FnOnce(&mut LedgerState) -> Result<T, ChainLedgerError>,
    ) -> Result<T, ChainLedgerError> {
        let (result, events) = {
            let mut state = self.state.lock().expect("agent chain ledger lock poisoned");
            let mut next = state.clone();
            let result = f(&mut next);
            let events = std::mem::take(&mut next.events);
            if next.dirty {
                let chains: Vec<&store::StoredChain> = next.chains.values().collect();
                store::write(&self.path, &chains)?;
                *state = next;
                self.bump_revision();
            }
            (result, events)
        };
        if let Some(handler) = self
            .on_limit
            .lock()
            .expect("agent chain ledger lock poisoned")
            .as_ref()
        {
            for event in events {
                handler(&event);
            }
        }
        result
    }

    fn bump_revision(&self) {
        self.revision.send_modify(|revision| *revision += 1);
    }

    fn read<T>(&self, f: impl FnOnce(&LedgerState) -> T) -> T {
        f(&self.state.lock().expect("agent chain ledger lock poisoned"))
    }
}

impl LedgerState {
    fn empty() -> Self {
        Self {
            default_limits: ChainLimits::default(),
            chains: BTreeMap::new(),
            sessions: HashMap::new(),
            wake: HashMap::new(),
            cards: HashMap::new(),
            activity: HashMap::new(),
            tick: 0,
            events: Vec::new(),
            dirty: false,
        }
    }
}

fn build_row(state: &LedgerState, chain: &store::StoredChain) -> AgentChainRow {
    let open = !matches!(
        chain.status,
        AgentChainStatus::Stopped | AgentChainStatus::Ended
    );
    AgentChainRow {
        live: open
            && state
                .sessions
                .values()
                .any(|entry| entry.live && entry.chain == chain.chain),
        chain: chain.chain.clone(),
        user: chain.user.clone(),
        origin: chain.origin.clone(),
        bots: chain
            .bots
            .iter()
            .map(|entry| AgentChainBot {
                bot: entry.bot.clone(),
                layer: entry.layer,
            })
            .collect(),
        rounds: chain.rounds,
        status: chain.status,
        can_stop: true,
    }
}
