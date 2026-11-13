//! 链用到上限的卡片（ADR-0061 第 10 节）：发给链代表的人和链上各 Bot 的归属人，同一人只发一张。
//! 全部选“再给一份”才放开（轮数和花费各加一份默认上限）；任一人选“先停着”就停整条链。
//!
//! 卡片由账本的“用到上限”回调在后台拉起：回调在账本锁外、同步调用，卡片在这里等回答。
//! 过期、撤不回、发不出都按“不再放行”处理——宁可停下让人重新说一句，也不悄悄继续烧。

use std::sync::Arc;

use futures_util::stream::{FuturesUnordered, StreamExt};
use wonton_protocol::{
    AgentChainRow, Answer, BotRef, CardChoice, ChainLimitKind, ConfirmationCategory,
    ConfirmationRequester,
};

use super::cards::{Draft, line};
use super::{Audience, Bus};
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
        asking.push(ask_one(
            bus,
            chains.clone(),
            &caller,
            &row,
            event,
            recipient,
        ));
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
    chains: Arc<crate::gateway::agent_chains::AgentChains>,
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
    if let Err(error) = chains.ledger().bind_card(&row.chain, &card.id) {
        tracing::warn!(%error, card = %card.id, "上限卡片没能认到链上");
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
        ChainLimitKind::Cost => l10n::text(langs, "chat-chain-limit-title-cost", &[]),
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
    }
}

/// 链停下时撤回链上还等着的卡片：等待方拿到 `cancelled`，卡片在列表里显示撤回原因。
pub(super) fn withdraw_cards(bus: &Bus, outcome: &crate::agent_chain::ChainStopOutcome) {
    for card in &outcome.cards {
        let langs = bus.languages_for(bus.confirmations.recipient(card).flatten().as_deref());
        let message = l10n::text(&langs, "chat-chain-stopped-card", &[]);
        if !bus.confirmations.cancel(card, &message) {
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
