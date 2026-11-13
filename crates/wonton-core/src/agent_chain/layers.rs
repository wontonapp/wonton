//! 层数、绕圈和自调（ADR-0061 第 10 节）。检查顺序固定：已停 → 到上限 → 自调 → 绕圈 → 层数，
//! 前面的命中就返回、不看后面。这些检查不翻链的状态；翻状态（轮数、花费）在 `budget` 里。

use wonton_protocol::{AgentChainStatus, BotRef, ChainError, ChainLimitKind, ChainRefusal};

use super::store::StoredChain;

pub(super) fn layer_of(chain: &StoredChain, bot: &BotRef) -> Option<u32> {
    chain
        .bots
        .iter()
        .find(|entry| &entry.bot == bot)
        .map(|entry| entry.layer)
}

/// 链已停或已到上限时，一切新登记（开轮、交接、连接）都拒。
pub(super) fn reject_if_closed(chain: &StoredChain) -> Result<(), ChainError> {
    match chain.status {
        AgentChainStatus::Stopped => Err(ChainError::Stopped),
        AgentChainStatus::LimitReached => Err(ChainError::LimitReached {
            kind: chain.limit_kind.unwrap_or(ChainLimitKind::Other),
        }),
        _ => Ok(()),
    }
}

/// 桥的一轮续到已有的链：Bot 已在链上就用自己的层（点名叫回沿用原层）；
/// 不在链上就是被点名的：进这条链、层同点名者，不加层（ADR-0061 第 10 节）。
/// 唤醒标签只登记链上 Bot 发的消息（第 8 块），走到这里的点名者一定在链上；不是就是调用方报错了。
pub(super) fn bridge_layer(
    chain: &StoredChain,
    bot: &BotRef,
    mentioner: Option<&BotRef>,
) -> Result<u32, ChainError> {
    match layer_of(chain, bot) {
        Some(layer) => Ok(layer),
        None => mentioner
            .and_then(|mentioner| layer_of(chain, mentioner))
            .ok_or(ChainError::InvalidInput {
                refusal: ChainRefusal::Other,
            }),
    }
}

/// Keep one summary row per Bot at its minimum admitted layer. Session layers, not these
/// presentation rows, determine recursion depth.
pub(super) fn admit_bot(chain: &mut StoredChain, bot: &BotRef, layer: u32) {
    if let Some(position) = chain.bots.iter().position(|entry| &entry.bot == bot) {
        if chain.bots[position].layer <= layer {
            return;
        }
        chain.bots.remove(position);
    }
    let position = chain.bots.partition_point(|entry| entry.layer <= layer);
    chain.bots.insert(
        position,
        super::store::ChainBotEntry {
            bot: bot.clone(),
            layer,
        },
    );
}

pub(super) fn next_layer(top: u32, depth: u32, max: u32) -> Result<u32, ChainError> {
    let previous = top.max(depth);
    if previous >= max {
        return Err(ChainError::LimitReached {
            kind: ChainLimitKind::Depth,
        });
    }
    Ok(previous + 1)
}
