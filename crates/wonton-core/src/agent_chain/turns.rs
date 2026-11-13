//! 桥的一轮怎么落到链上（ADR-0061 第 10 节，第 8 块的叫回在这接）：续到已有链还是开新链、
//! 记轮数、登记会话。检查和记账在 `layers`、`budget`，这里只管顺序和登记。

use wonton_protocol::{AgentChainStatus, BotRef, ChainTurnGrant, ConfirmationOrigin};

use super::store;
use super::{ChainLedgerError, ChainSessionSource, LedgerState, SessionEntry};
use super::{budget, layers, stop};

/// 桥的一轮续到已有的链：检查（已停、到上限）→ 记轮数（到轮数上限的那一下就地翻转）→
/// 登记桥会话。被拒的轮不登记会话，但翻转要提交——"再给一份"靠它。
/// 被点名的 Bot 不在链上时用点名者的层进链（不加层，轮数照记）。
pub(super) fn continue_bridge_round(
    state: &mut LedgerState,
    chain_id: &str,
    bot: &BotRef,
    session: &str,
    sender: &str,
    mentioner: Option<&BotRef>,
) -> Result<ChainTurnGrant, ChainLedgerError> {
    let (layer, new_bot) = {
        let chain = state
            .chains
            .get(chain_id)
            .ok_or_else(|| ChainLedgerError::UnknownChain(chain_id.to_owned()))?;
        layers::reject_if_closed(chain).map_err(ChainLedgerError::Rejected)?;
        let layer = match state.sessions.get(session) {
            Some(entry) if entry.chain == chain_id && entry.bot == *bot => entry.layer,
            _ => layers::bridge_layer(chain, bot, mentioner).map_err(ChainLedgerError::Rejected)?,
        };
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
    let parents = match state.sessions.get(session) {
        Some(entry) if entry.chain == chain_id && entry.bot == *bot => entry.path.parents.clone(),
        _ => super::ancestry::callers(state, chain_id, mentioner, None),
    };
    {
        let chain = state.chains.get_mut(chain_id).expect("链刚查过");
        budget::count_round(chain, &mut state.events).map_err(ChainLedgerError::Rejected)?;
        // 被拒的轮不加轮数也不入列；走到这里的 Bot 已经过了层数检查，这轮记上了它才入列。
        if new_bot {
            layers::admit_bot(chain, bot, layer);
        }
    }
    bind_session(
        state,
        chain_id,
        session,
        store::ChainBotEntry {
            bot: bot.clone(),
            layer,
        },
        ChainSessionSource::Bridge,
        Some(sender),
        parents,
    )?;
    stop::bump_activity(state, chain_id);
    Ok(ChainTurnGrant {
        chain: chain_id.to_owned(),
        layer,
    })
}

/// 开一条新链：首个 Bot 的层由入口核对后给出。桥的轮在开链那刻记一轮（start 就是 prompt 的开始）；
/// `agent_acp` 的轮等 prompt 来了再记。
/// `user` 是链代表的人；`sender` 是实际说话人，点名的 Bot 说话时两者不同。
pub(super) fn open_chain(
    state: &mut LedgerState,
    user: String,
    sender: &str,
    origin: ConfirmationOrigin,
    first: store::ChainBotEntry,
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
    let sender = (source == ChainSessionSource::Bridge).then(|| sender.to_owned());
    let mut chain = store::StoredChain {
        chain: chain_id.clone(),
        user,
        origin,
        bots: Vec::new(),
        limits: state.default_limits,
        rounds: 0,
        tokens: 0,
        amount: None,
        spend_by_bot: Vec::new(),
        status: AgentChainStatus::Running,
        limit_kind: None,
        limit_events: 0,
    };
    let store::ChainBotEntry { bot, layer } = first;
    layers::admit_bot(&mut chain, &bot, layer);
    if source == ChainSessionSource::Bridge {
        budget::count_round(&mut chain, &mut state.events).map_err(ChainLedgerError::Rejected)?;
    }
    state.chains.insert(chain_id.clone(), chain);
    state.dirty = true;
    bind_session(
        state,
        &chain_id,
        session,
        store::ChainBotEntry {
            bot: bot.clone(),
            layer,
        },
        source,
        sender.as_deref(),
        Vec::new(),
    )?;
    stop::bump_activity(state, &chain_id);
    Ok(ChainTurnGrant {
        chain: chain_id,
        layer,
    })
}

/// 登记会话。桥会话每轮重新登记、覆盖旧登记（上一轮必须已报 end）；
/// `agent_acp` 和交接会话一登记就定死，重复登记同一对（链、Bot）按幂等处理。
fn bind_session(
    state: &mut LedgerState,
    chain_id: &str,
    session: &str,
    first: store::ChainBotEntry,
    source: ChainSessionSource,
    sender: Option<&str>,
    parents: Vec<std::sync::Arc<super::ancestry::SessionPath>>,
) -> Result<(), ChainLedgerError> {
    let store::ChainBotEntry { bot, layer } = first;
    let entry = SessionEntry {
        chain: chain_id.to_owned(),
        bot: bot.clone(),
        layer,
        source,
        path: std::sync::Arc::new(super::ancestry::SessionPath {
            session: session.to_owned(),
            bot: bot.clone(),
            layer,
            parents,
        }),
        connection: false,
        sender: sender.map(str::to_owned),
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
