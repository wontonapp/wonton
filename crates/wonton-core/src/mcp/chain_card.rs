//! 链卡片：用到上限的卡片（ADR-0061 第 10 节）和卡片认链的公共逻辑。
//!
//! 上限卡片发给链代表的人和链上各 Bot 的归属人，同一人只发一张。
//! 全部选“再给一份”才放开（轮数加一份默认上限）；任一人选“先停着”就停整条链。
//!
//! 卡片由账本的“用到上限”回调在后台拉起：回调在账本锁外、同步调用，卡片在这里等回答。
//! 过期、撤不回、发不出都按“不再放行”处理——宁可停下让人重新说一句，也不悄悄继续烧。

use std::sync::Arc;

use futures_util::stream::{FuturesUnordered, StreamExt};
use wonton_protocol::{
    AgentChainRow, AgentChainStatus, Answer, BotRef, CardChoice, ChainLimitKind,
    ConfirmationCategory, ConfirmationRequester, ConfirmationState,
};

use super::cards::{Draft, line};
use super::{Audience, Bus, Caller, TurnSender};
use crate::agent_chain::ChainLimitEvent;
use crate::l10n;
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;

/// 账本的上限回调：后台发卡片，等所有人的回答。
pub fn spawn(bus: Arc<Bus>, event: ChainLimitEvent) {
    tokio::spawn(async move {
        if let Err(error) = send(&bus, &event).await {
            tracing::warn!(chain = %event.chain, %error, "上限卡片没发出去");
        }
    });
}

async fn send(bus: &Arc<Bus>, event: &ChainLimitEvent) -> Result<(), SendError> {
    let chains = bus.agent_chains().ok_or(SendError::NotAttached)?;
    let Some(row) = chains.ledger().row(&event.chain) else {
        return Err(SendError::UnknownChain(event.chain.clone()));
    };
    let Some(first) = row.bots.first() else {
        return Err(SendError::NoBots(event.chain.clone()));
    };
    let caller =
        PluginId::parse(&first.bot.plugin_id).map_err(|e| SendError::BadPlugin(e.to_string()))?;
    let recipients = recipients(bus, &row);
    if recipients.is_empty() {
        return Err(SendError::NoRecipients(event.chain.clone()));
    }
    let total = recipients.len();
    let mut asking = FuturesUnordered::new();
    for recipient in recipients {
        asking.push(ask_one(bus, &caller, &row, event, recipient));
    }
    let mut approved = 0;
    while let Some(answered) = asking.next().await {
        if answered {
            approved += 1;
            if approved == total {
                if let Err(error) = chains.ledger().grant_another(&event.chain) {
                    tracing::warn!(chain = %event.chain, %error, "链放开失败");
                }
                return Ok(());
            }
            continue;
        }
        // 任一人不放行：停整条链，剩下还没答的卡片随停链撤回。
        if let Err(error) = bus.stop_chain(&event.chain).await {
            tracing::warn!(chain = %event.chain, %error, "链停下失败");
        }
        return Ok(());
    }
    Ok(())
}

/// 问一个人，返回他是否放行。
async fn ask_one(
    bus: &Arc<Bus>,
    caller: &PluginId,
    row: &AgentChainRow,
    event: &ChainLimitEvent,
    recipient: Option<String>,
) -> bool {
    let langs = bus.languages_for(recipient.as_deref());
    let draft = draft(row, event.kind, &langs);
    let (card, answered) = match bus.confirmations().open_as(
        caller,
        draft,
        Audience {
            on_behalf: None,
            recipients: super::Recipients::Keeper(recipient),
            requester: Some(ConfirmationRequester::Bot {
                bot: row.bots[0].bot.clone(),
                user_id: bot_user(bus, &row.bots[0].bot),
            }),
            origin: Some(row.origin.clone()),
            category: None,
        },
    ) {
        Ok(opened) => opened,
        Err(refusal) => {
            tracing::warn!(kind = refusal.kind(), chain = %row.chain, "上限卡片没能开出来");
            return false;
        }
    };
    // 链已经停了（另一张卡片先选了“先停着”）就不再等。
    if bind_chain(bus, &row.chain, &card.id, "上限卡片").outcome == Bound::Withdrawn {
        return false;
    }
    match bus.confirmations().deliver(&card, answered).await {
        Ok(chosen) => chosen.answer == Answer::Approve,
        Err(_) => false,
    }
}

/// 该发给谁：链代表的人和链上各 Bot 的归属人。主人在协议里是 `None`，同一人只留一份。
fn recipients(bus: &Bus, row: &AgentChainRow) -> Vec<Option<String>> {
    let owner = bus.plugins.owner_name();
    let mut people: Vec<Option<String>> = Vec::new();
    let mut push = |username: Option<String>| {
        let recipient = username.filter(|name| name.as_str() != owner);
        if !people.contains(&recipient) {
            people.push(recipient);
        }
    };
    push(username_of(bus, &row.user));
    for entry in &row.bots {
        push(keeper_of(bus, &entry.bot));
    }
    people
}

fn username_of(bus: &Bus, user_id: &str) -> Option<String> {
    let rest = user_id.strip_prefix('@')?;
    let (localpart, server) = rest.split_once(':')?;
    let own = bus.plugins.matrix().ok()?.server_name().to_owned();
    (server == own).then(|| localpart.to_owned())
}

fn bot_user(bus: &Bus, bot: &BotRef) -> String {
    let Some(id) = PluginId::parse(&bot.plugin_id).ok() else {
        return String::new();
    };
    bus.plugins
        .bot_account(&id, &bot.bot)
        .map(|account| account.user_id)
        .unwrap_or_default()
}

fn keeper_of(bus: &Bus, bot: &BotRef) -> Option<String> {
    let id = PluginId::parse(&bot.plugin_id).ok()?;
    let account = bus.plugins.bot_account(&id, &bot.bot)?;
    bus.plugins
        .bot_by_user(&account.user_id)
        .map(|(_, _, keeper)| keeper)
}

fn draft(row: &AgentChainRow, kind: ChainLimitKind, langs: &Languages) -> Draft {
    let title = match kind {
        ChainLimitKind::Rounds => l10n::text(langs, "chat-chain-limit-title-rounds", &[]),
        ChainLimitKind::Depth | ChainLimitKind::Other => {
            l10n::text(langs, "chat-chain-limit-title", &[])
        }
    };
    let bots = row
        .bots
        .iter()
        .map(|entry| entry.bot.bot.clone())
        .collect::<Vec<_>>()
        .join("、");
    Draft {
        title,
        lines: vec![
            line(l10n::text(
                langs,
                "chat-chain-limit-bots",
                &[("bots", bots.as_str().into())],
            )),
            line(l10n::text(langs, "chat-chain-limit-what", &[])),
        ],
        choices: vec![
            CardChoice {
                answer: Answer::Approve,
                label: l10n::text(langs, "chat-chain-limit-grant", &[]),
                choice_id: None,
            },
            CardChoice {
                answer: Answer::Reject,
                label: l10n::text(langs, "chat-chain-limit-stop", &[]),
                choice_id: None,
            },
        ],
        scope: None,
        plugin_id: row.bots.first().map(|entry| entry.bot.plugin_id.clone()),
        secret_inputs: Vec::new(),
        settings: None,
        category: Some(ConfirmationCategory::ChainLimitReached {
            chain: row.chain.clone(),
            kind,
        }),
        permissions: Vec::new(),
        devices: Vec::new(),
    }
}

/// 卡片认链的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Bound {
    /// 卡片在链上等回答。
    Waiting,
    /// 停链抢在绑卡之前：卡片刚被撤回，调用方按 [`super::Refusal::Cancelled`] 回、不要执行。
    Withdrawn,
    /// 没绑上（没有链、链服务没接上或账本出错）：卡片照常等，只是停链撤回不了它。
    Unbound,
}

/// [`bind_card`] 的结果：认到哪条链、卡片落成什么。
pub(super) struct Binding {
    pub outcome: Bound,
    /// 绑上的链；没绑上时是 `None`。调用方要按它回写卡片所属的链。
    pub chain: Option<String>,
}

impl Binding {
    fn unbound() -> Self {
        Self {
            outcome: Bound::Unbound,
            chain: None,
        }
    }
}

/// 这次调用认得到哪条链：调用头报的那条（[`Caller::chain`]）；没报时按这个 Bot 最近活着、
/// 说话人对得上的一轮认（[`crate::agent_chain::Chains::live_turn_chain`]）。
/// 认不出是 `Ok(None)`；链服务没接上分开报 `Err(())`。
fn caller_chain(bus: &Bus, caller: &Caller) -> Result<Option<String>, ()> {
    let chains = bus.agent_chains().ok_or(())?;
    if let Some(chain) = &caller.chain {
        return Ok(Some(chain.clone()));
    }
    let sender = match &caller.sender {
        Some(TurnSender::Local(sender)) => Some(sender.as_str()),
        _ => None,
    };
    let Some(bot) = caller.bot.as_ref() else {
        return Ok(None);
    };
    Ok(chains
        .ledger()
        .live_turn_chain(caller.id.as_str(), &bot.bot, sender))
}

/// 把卡片认到 `chain`（给 `Some` 时）或这次调用认得到的链上，见 [`bind_chain`]。
///
/// `what` 是日志里的卡片类别。认不出链时卡片照发，只是停链撤回不了它。
pub(super) fn bind_card(
    bus: &Bus,
    caller: &Caller,
    chain: Option<&str>,
    card: &str,
    what: &str,
) -> Binding {
    let chain = match chain {
        Some(chain) => Some(chain.to_owned()),
        None => match caller_chain(bus, caller) {
            Ok(chain) => chain,
            Err(()) => {
                tracing::warn!(card = %card, "调用链的服务没接上，{what}不入链：停链撤回不了它");
                return Binding::unbound();
            }
        },
    };
    match chain {
        Some(chain) => bind_chain(bus, &chain, card, what),
        None => {
            tracing::warn!(card = %card, "{what}认不出是哪一轮，卡片不入链：停链撤回不了它");
            Binding::unbound()
        }
    }
}

/// 把卡片绑到链上，绑上之后回查一眼链状态。
///
/// 停链和在开卡片、绑卡片的空隙里插进来时，`stop()` 已经收完待撤的卡片，没人再来撤这一张：
/// 这里自己撤回，别让它挂在“等你确认”、批准了还执行。
pub(super) fn bind_chain(bus: &Bus, chain: &str, card: &str, what: &str) -> Binding {
    let Some(chains) = bus.agent_chains() else {
        tracing::warn!(card = %card, %chain, "调用链的服务没接上，{what}不入链：停链撤回不了它");
        return Binding::unbound();
    };
    let ledger = chains.ledger();
    if let Err(error) = ledger.bind_card(chain, card) {
        tracing::warn!(%error, card = %card, %chain, "{what}没能认到链上");
        return Binding::unbound();
    }
    let chain = chain.to_owned();
    if ledger.status(&chain) != Some(AgentChainStatus::Stopped) {
        return Binding {
            outcome: Bound::Waiting,
            chain: Some(chain),
        };
    }
    let _ = ledger.resolve_card(card);
    let outcome = if withdraw_card(bus, card) {
        Bound::Withdrawn
    } else {
        // 撤不到：人抢在撤回之前答了（批了照常执行），或停链那一遍已经比这里先撤过。
        // 卡片已经是 `Failed` 就是后者：等待方拿到 `Cancelled`，不再给它发一遍通知。
        match bus.confirmations.card(card).map(|card| card.state) {
            Some(ConfirmationState::Failed { .. }) => Bound::Withdrawn,
            _ => Bound::Waiting,
        }
    };
    Binding {
        outcome,
        chain: Some(chain),
    }
}

/// 撤回一张卡片：等待方拿到 `cancelled`，卡片在列表里显示撤回原因。已经不在等时返回假。
fn withdraw_card(bus: &Bus, card: &str) -> bool {
    let langs = bus.languages_for(bus.confirmations.recipient(card).flatten().as_deref());
    let message = l10n::text(&langs, "chat-chain-stopped-card", &[]);
    bus.confirmations.cancel(card, &message)
}

/// 链停下时撤回链上还等着的卡片。
pub(super) fn withdraw_cards(bus: &Bus, outcome: &crate::agent_chain::ChainStopOutcome) {
    for card in &outcome.cards {
        if !withdraw_card(bus, card) {
            tracing::info!(card = %card, "卡片已经不在等，不用撤回");
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum SendError {
    #[error("链的服务没接上")]
    NotAttached,
    #[error("没有链 {0}")]
    UnknownChain(String),
    #[error("链 {0} 上没有 Bot")]
    NoBots(String),
    #[error("链 {0} 没有任何收件人")]
    NoRecipients(String),
    #[error("插件 ID 认不出来：{0}")]
    BadPlugin(String),
}
