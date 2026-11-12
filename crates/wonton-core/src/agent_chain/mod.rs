//! Agent 调用链账本（ADR-0061 第 10 节，第 2 块）：一句话引出的所有 Agent 会话是一条链，
//! 链上记身份、层数、轮数和花费，随时能停整条链。纯逻辑，不接 HTTP、桥和 MCP 工具
//! （第 3、6、7 块经这里的公开方法接）；发起人权限、房间表判断（#248）也归它们。
//! 线上类别（[`ChainError`] 等）用 wonton-protocol 已有的；账本自己的类型
//! （来源、会话信息、停链结果、上限事件）只在核心内部第 2/3/6/8 块之间用，不上线。

mod budget;
mod layers;
mod stop;
mod store;
#[cfg(test)]
mod tests;
mod wake;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use wonton_protocol::{
    AgentChainBot, AgentChainRow, AgentChainStatus, AgentChainsViewModel, BotRef, ChainError,
    ChainLimitKind, ChainLimits, ChainSpend, ChainTurnGrant, ChainTurnReport, ConfirmationOrigin,
};

/// 会话的来源（ADR-0061 第 10 节）：桥的一轮、`agent_acp` 连接上的会话、交接开出的会话。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainSessionSource {
    Bridge,
    AgentAcp,
    HandOff,
}

/// 账本里一个会话的登记：会话 ID → 链、Bot、层、来源。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainSessionInfo {
    pub session: String,
    pub chain: String,
    pub bot: BotRef,
    pub layer: u32,
    pub source: ChainSessionSource,
    /// 还活着：没报结束、没被停链取消。重启后一律为假。
    pub live: bool,
}

/// 停链的交出项：要取消的活会话按来源分，加上带这条链、还没答的卡片（要撤回）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChainStopOutcome {
    pub bridge: Vec<ChainSessionInfo>,
    pub agent_acp: Vec<ChainSessionInfo>,
    pub hand_off: Vec<ChainSessionInfo>,
    pub cards: Vec<String>,
}

/// 链到上限的那一下（轮数或花费）。每次进入 `limit_reached` 恰好发出一次；
/// 层数不算——层数超了只拒那一跳，不翻链的状态，也没有"再给一份"。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainLimitEvent {
    pub chain: String,
    pub kind: ChainLimitKind,
}

/// 账本操作的错误。桥和网关要回给调用方的拒绝包在 [`ChainLedgerError::Rejected`]（线上类别
/// 原样透传）；其余是账本自己的状态错误和落盘失败。
#[derive(Debug, thiserror::Error)]
pub enum ChainLedgerError {
    #[error("链拒绝：{0:?}")]
    Rejected(ChainError),
    #[error("链 {0} 不存在")]
    UnknownChain(String),
    #[error("会话 {0} 没有登记在账本里")]
    UnknownSession(String),
    #[error("卡片 {0} 没有登记在账本里")]
    UnknownCard(String),
    #[error("链 {chain} 已经结束，不能再停")]
    AlreadyEnded { chain: String },
    #[error("链 {chain} 不在 limit_reached，给不了新的一份")]
    NotAtLimit { chain: String },
    #[error("链 {chain} 还有活着的会话，不能记成做完")]
    SessionsLive { chain: String },
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
    source: ChainSessionSource,
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

    /// 桥的一轮开始（`POST /agent/chains` 的 start 报告）：按触发消息的唤醒标签沿用原链，
    /// 沿用不上就开新链（发起的 Bot 记在第 1 层）。拿到 [`ChainTurnGrant`] 才发 prompt，
    /// 轮数在拿到的那一刻记。说话人是不是这个 Bot 听的人归第 3 块核，账本只记。
    pub fn start_bridge_turn(
        &self,
        bot: BotRef,
        report: ChainTurnReport,
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
                "start_bridge_turn 只收 start 报告；end 走 end_session".into(),
            ));
        };
        self.mutate(|state| {
            let origin = ConfirmationOrigin {
                room_id: room,
                thread_id: thread,
            };
            match wake::lookup(state, &event_id) {
                Some(chain_id) => continue_bridge_round(state, &chain_id, &bot, &session),
                None => open_chain(
                    state,
                    sender,
                    origin,
                    bot,
                    &session,
                    ChainSessionSource::Bridge,
                ),
            }
        })
    }

    /// 一轮结束（桥的 end 报告、网关侧会话完结）。End 对被拒的轮也会报，
    /// 没登记的会话静默忽略；不改链的状态（"做完"由 [`Chains::end_chain`] 显式记）。
    pub fn end_session(&self, session: &str) {
        let mut state = self.state.lock().expect("agent chain ledger lock poisoned");
        if let Some(entry) = state.sessions.get_mut(session) {
            entry.live = false;
        }
    }

    /// 交接或 `agent_acp` 连接的预检（第 6/3 块在开远端会话/连接前调，省得开了再取消）：
    /// 检查顺序固定（已停 → 到上限 → 自调 → 绕圈 → 层数），返回目标会落在的层。不改任何状态。
    pub fn preview_attach(
        &self,
        chain_id: &str,
        target: &BotRef,
        initiator: Option<&BotRef>,
    ) -> Result<u32, ChainLedgerError> {
        self.read(|state| {
            let chain = state
                .chains
                .get(chain_id)
                .ok_or_else(|| ChainLedgerError::UnknownChain(chain_id.to_owned()))?;
            layers::evaluate_attach(chain, target, initiator).map_err(ChainLedgerError::Rejected)
        })
    }

    /// 登记 `agent_acp` 连接上或交接开出的会话，返回它的层。登记前重跑全部检查
    /// （预检和登记之间链可能被并发改动），登记时撞上拒绝就由调用方取消刚开的远端会话。
    /// 同一对（链、Bot）重复登记按幂等处理（断线重连的 `session/load`），返回原层。
    pub fn register_session(
        &self,
        chain_id: &str,
        session: &str,
        bot: BotRef,
        initiator: Option<&BotRef>,
        source: ChainSessionSource,
    ) -> Result<u32, ChainLedgerError> {
        if source == ChainSessionSource::Bridge {
            return Err(ChainLedgerError::Invalid(
                "桥的会话经 start_bridge_turn 登记".into(),
            ));
        }
        self.mutate(|state| {
            {
                let chain = state
                    .chains
                    .get(chain_id)
                    .ok_or_else(|| ChainLedgerError::UnknownChain(chain_id.to_owned()))?;
                layers::reject_if_closed(chain).map_err(ChainLedgerError::Rejected)?;
            }
            if let Some(existing) = state.sessions.get_mut(session) {
                if existing.chain == chain_id && existing.bot == bot {
                    // session/load 重连：会话又活了，停链时照样要取消它。
                    existing.live = true;
                    return Ok(existing.layer);
                }
                return Err(ChainLedgerError::Invalid(format!(
                    "会话 {session} 已登记在别的链上"
                )));
            }
            let layer = {
                let chain = state
                    .chains
                    .get_mut(chain_id)
                    .ok_or_else(|| ChainLedgerError::UnknownChain(chain_id.to_owned()))?;
                let layer = layers::evaluate_attach(chain, &bot, initiator)
                    .map_err(ChainLedgerError::Rejected)?;
                // evaluate_attach 拦下了绕圈，走到这里的 Bot 一定还不在链上：入列（层序即登记序）。
                chain.bots.push(store::ChainBotEntry {
                    bot: bot.clone(),
                    layer,
                });
                layer
            };
            state.dirty = true;
            state.sessions.insert(
                session.to_owned(),
                SessionEntry {
                    chain: chain_id.to_owned(),
                    bot: bot.clone(),
                    layer,
                    source,
                    live: true,
                },
            );
            stop::bump_activity(state, chain_id);
            Ok(layer)
        })
    }

    /// 给网页类前端开一条新链（ADR-0061 第 10 节：核心在建连接时发链 ID）：
    /// 目标 Bot 是第 1 层并登记这个会话。开链不记轮数——轮数跟着 prompt 走（[`Chains::begin_round`]）。
    pub fn open_acp_chain(
        &self,
        user: &str,
        origin: ConfirmationOrigin,
        target: BotRef,
        session: &str,
    ) -> Result<ChainTurnGrant, ChainLedgerError> {
        self.mutate(|state| {
            open_chain(
                state,
                user.to_owned(),
                origin,
                target,
                session,
                ChainSessionSource::AgentAcp,
            )
        })
    }

    /// 记一个 `session/prompt`（`agent_acp` 和交接会话的轮数；桥的轮数在 start 里记）。
    /// 已停或已到上限的链先于会话死活被拒：停链之后开轮拿到 `stopped`。
    pub fn begin_round(&self, session: &str) -> Result<(), ChainLedgerError> {
        self.mutate(|state| {
            let chain_id = state
                .sessions
                .get(session)
                .ok_or_else(|| ChainLedgerError::UnknownSession(session.to_owned()))?
                .chain
                .clone();
            let chain = state.chains.get_mut(&chain_id).expect("会话的链一定登记过");
            layers::reject_if_closed(chain).map_err(ChainLedgerError::Rejected)?;
            if !state.sessions.get(session).expect("刚查过").live {
                return Err(ChainLedgerError::Invalid(format!(
                    "会话 {session} 已经结束"
                )));
            }
            state.dirty = true;
            budget::count_round(chain, &mut state.events).map_err(ChainLedgerError::Rejected)
        })
    }

    /// 记模型网关报来的用量（按 Bot 分账，链合计同时加）。到花费上限的那一下把链置
    /// `LimitReached` 并发一次"用到上限"（见 [`Chains::on_limit_event`]）；
    /// 已停或已到上限的链只记账，不再翻转。
    pub fn record_tokens(
        &self,
        chain_id: &str,
        bot: &BotRef,
        tokens: u64,
        amount: Option<f64>,
    ) -> Result<(), ChainLedgerError> {
        self.mutate(|state| {
            {
                let chain = state
                    .chains
                    .get_mut(chain_id)
                    .ok_or_else(|| ChainLedgerError::UnknownChain(chain_id.to_owned()))?;
                budget::record_tokens(chain, bot, tokens, amount, &mut state.events);
            }
            state.dirty = true;
            Ok(())
        })
    }

    /// 停整条链：置 `Stopped`，交出链上活的会话（按来源分）和待答的卡片，
    /// 之后一切新登记都是 `stopped`。已停的链再停返回空结果（幂等）；做完的链不能停。
    pub fn stop(&self, chain_id: &str) -> Result<ChainStopOutcome, ChainLedgerError> {
        self.mutate(|state| stop::stop(state, chain_id))
    }

    /// "再给一份"（上限卡片的按钮）：轮数、花费上限各加一份默认值，回到 `Running`。
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

    /// 把链记成做完（"做完了"的判断归第 3/8 块，账本只收显式标记）。
    /// 还有活会话时拒绝——做完的链不该有没取消的会话。摘掉这条链的唤醒标签。
    pub fn end_chain(&self, chain_id: &str) -> Result<(), ChainLedgerError> {
        self.mutate(|state| {
            if state
                .sessions
                .values()
                .any(|entry| entry.chain == chain_id && entry.live)
            {
                return Err(ChainLedgerError::SessionsLive {
                    chain: chain_id.to_owned(),
                });
            }
            let chain = state
                .chains
                .get_mut(chain_id)
                .ok_or_else(|| ChainLedgerError::UnknownChain(chain_id.to_owned()))?;
            if chain.status == AgentChainStatus::Ended {
                return Err(ChainLedgerError::Invalid(format!("链 {chain_id} 已经结束")));
            }
            chain.status = AgentChainStatus::Ended;
            chain.limit_kind = None;
            state.dirty = true;
            wake::drop_chain(state, chain_id);
            Ok(())
        })
    }

    /// 登记一张带这条链的卡片（权限卡片、上限卡片），停链时随活会话一起交出撤回。
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

    /// 按消息事件找链（点名叫回沿用原链，第 8 块用）。
    pub fn chain_for_event(&self, event_id: &str) -> Option<String> {
        self.read(|state| wake::lookup(state, event_id))
    }

    /// 按会话找链。
    pub fn session_chain(&self, session: &str) -> Option<ChainSessionInfo> {
        self.read(|state| {
            state.sessions.get(session).map(|entry| ChainSessionInfo {
                session: session.to_owned(),
                chain: entry.chain.clone(),
                bot: entry.bot.clone(),
                layer: entry.layer,
                source: entry.source,
                live: entry.live,
            })
        })
    }

    /// 链上登记过的全部会话（含已结束的，`live` 区分）。
    pub fn chain_sessions(&self, chain_id: &str) -> Vec<ChainSessionInfo> {
        self.read(|state| {
            state
                .sessions
                .iter()
                .filter(|(_, entry)| entry.chain == chain_id)
                .map(|(session, entry)| ChainSessionInfo {
                    session: session.clone(),
                    chain: entry.chain.clone(),
                    bot: entry.bot.clone(),
                    layer: entry.layer,
                    source: entry.source,
                    live: entry.live,
                })
                .collect()
        })
    }

    /// 链的按 Bot 分账，各项相加等于链的总 token。
    pub fn spend_by_bot(&self, chain_id: &str) -> Option<Vec<(BotRef, u64)>> {
        self.read(|state| {
            state.chains.get(chain_id).map(|chain| {
                chain
                    .spend_by_bot
                    .iter()
                    .map(|entry| (entry.bot.clone(), entry.tokens))
                    .collect()
            })
        })
    }

    /// App 的链列表：没结束的按最近活动排；停了和做完的不进列表，留给详情（[`Chains::row`]）。
    /// `can_stop` 恒为真——哪些人看得见、能停（链代表和各 Bot 的归属人）由第 3 块按归属过滤。
    pub fn view(&self) -> AgentChainsViewModel {
        self.read(|state| {
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
                    (tick, build_row(chain))
                })
                .collect();
            rows.sort_by_key(|(tick, _)| std::cmp::Reverse(*tick));
            AgentChainsViewModel {
                chains: rows.into_iter().map(|(_, row)| row).collect(),
            }
        })
    }

    /// 单条链（任意状态，App 的链详情用）。
    pub fn row(&self, chain_id: &str) -> Option<AgentChainRow> {
        self.read(|state| state.chains.get(chain_id).map(build_row))
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

/// 桥的一轮续到已有的链：检查（已停、到上限）→ 记轮数（到轮数上限的那一下就地翻转）→
/// 登记桥会话。被拒的轮不登记会话，但翻转要提交——"再给一份"靠它。
fn continue_bridge_round(
    state: &mut LedgerState,
    chain_id: &str,
    bot: &BotRef,
    session: &str,
) -> Result<ChainTurnGrant, ChainLedgerError> {
    let (layer, new_bot) = {
        let chain = state
            .chains
            .get(chain_id)
            .ok_or_else(|| ChainLedgerError::UnknownChain(chain_id.to_owned()))?;
        layers::reject_if_closed(chain).map_err(ChainLedgerError::Rejected)?;
        let layer = layers::bridge_layer(chain, bot).map_err(ChainLedgerError::Rejected)?;
        (layer, layers::layer_of(chain, bot).is_none())
    };
    // 同一桥会话的重复 start（响应丢了重试）：幂等重发，不重复记轮。
    if let Some(existing) = state.sessions.get(session)
        && existing.source == ChainSessionSource::Bridge
        && existing.chain == chain_id
        && existing.bot == *bot
        && existing.live
    {
        return Ok(ChainTurnGrant {
            chain: chain_id.to_owned(),
            layer: existing.layer,
        });
    }
    state.dirty = true;
    {
        let chain = state.chains.get_mut(chain_id).expect("链刚查过");
        budget::count_round(chain, &mut state.events).map_err(ChainLedgerError::Rejected)?;
        // 被拒的轮不加轮数也不入列；走到这里的 Bot 已经过了层数检查，这轮记上了它才入列。
        if new_bot {
            chain.bots.push(store::ChainBotEntry {
                bot: bot.clone(),
                layer,
            });
        }
    }
    bind_session(
        state,
        chain_id,
        session,
        bot,
        layer,
        ChainSessionSource::Bridge,
    )?;
    stop::bump_activity(state, chain_id);
    Ok(ChainTurnGrant {
        chain: chain_id.to_owned(),
        layer,
    })
}

/// 开一条新链：发起的 Bot 记在第 1 层。桥的轮在开链那刻记一轮（start 就是 prompt 的开始）；
/// `agent_acp` 的轮等 prompt 来了再记。开链不可能撞上限检查（新链是空的）。
fn open_chain(
    state: &mut LedgerState,
    user: String,
    origin: ConfirmationOrigin,
    bot: BotRef,
    session: &str,
    source: ChainSessionSource,
) -> Result<ChainTurnGrant, ChainLedgerError> {
    if let Some(existing) = state.sessions.get(session) {
        // 桥会话每轮重新登记（同一会话的下一句话是另一条链，start 的契约）；
        // 上一轮没报 end 就来登记下一轮，或 `agent_acp` 会话想换链，都是调用方的错。
        if source != ChainSessionSource::Bridge || existing.live {
            return Err(ChainLedgerError::Invalid(format!(
                "会话 {session} 已登记在别的链上"
            )));
        }
    }
    let chain_id = new_chain_id();
    let mut chain = store::StoredChain {
        chain: chain_id.clone(),
        user,
        origin,
        bots: vec![store::ChainBotEntry {
            bot: bot.clone(),
            layer: 1,
        }],
        limits: state.default_limits,
        rounds: 0,
        tokens: 0,
        amount: None,
        spend_by_bot: Vec::new(),
        status: AgentChainStatus::Running,
        limit_kind: None,
        limit_events: 0,
    };
    if source == ChainSessionSource::Bridge {
        budget::count_round(&mut chain, &mut state.events).map_err(ChainLedgerError::Rejected)?;
    }
    state.chains.insert(chain_id.clone(), chain);
    state.dirty = true;
    bind_session(state, &chain_id, session, &bot, 1, source)?;
    stop::bump_activity(state, &chain_id);
    Ok(ChainTurnGrant {
        chain: chain_id,
        layer: 1,
    })
}

/// 登记会话。桥会话每轮重新登记、覆盖旧登记（上一轮必须已报 end）；
/// `agent_acp` 和交接会话一登记就定死，重复登记同一对（链、Bot）按幂等处理。
fn bind_session(
    state: &mut LedgerState,
    chain_id: &str,
    session: &str,
    bot: &BotRef,
    layer: u32,
    source: ChainSessionSource,
) -> Result<(), ChainLedgerError> {
    let entry = SessionEntry {
        chain: chain_id.to_owned(),
        bot: bot.clone(),
        layer,
        source,
        live: true,
    };
    match state.sessions.get(session) {
        Some(existing) if source == ChainSessionSource::Bridge => {
            if existing.live {
                return Err(ChainLedgerError::Invalid(format!(
                    "会话 {session} 的上一轮还没报 end"
                )));
            }
        }
        Some(_) => {
            return Err(ChainLedgerError::Invalid(format!(
                "会话 {session} 已登记在别的链上"
            )));
        }
        None => {}
    }
    state.sessions.insert(session.to_owned(), entry);
    Ok(())
}

/// 128 位随机的链 ID（ADR-0061 第 10 节），32 个十六进制字符。
fn new_chain_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("操作系统随机数源不可用");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn build_row(chain: &store::StoredChain) -> AgentChainRow {
    AgentChainRow {
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
        spend: if chain.tokens > 0 {
            ChainSpend::ViaGateway {
                tokens: chain.tokens,
                amount: chain.amount,
            }
        } else {
            ChainSpend::Unknown
        },
        status: chain.status,
        can_stop: true,
    }
}
