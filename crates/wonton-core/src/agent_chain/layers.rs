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

/// 链上的最高层；空链不会出现（开链就登记发起的 Bot），防御性返回 0。
fn top_layer(chain: &StoredChain) -> u32 {
    chain.bots.last().map_or(0, |entry| entry.layer)
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

/// 把 `target` 加进链（交接、`agent_acp` 连接）要过的全部检查，返回它的层。
/// `initiator` 是发起的 Bot；网页前端直连时没有，自调检查随之跳过。
pub(super) fn evaluate_attach(
    chain: &StoredChain,
    target: &BotRef,
    initiator: Option<&BotRef>,
) -> Result<u32, ChainError> {
    reject_if_closed(chain)?;
    if initiator == Some(target) {
        return Err(ChainError::InvalidInput {
            refusal: ChainRefusal::SelfCall,
        });
    }
    if layer_of(chain, target).is_some() {
        return Err(ChainError::InvalidInput {
            refusal: ChainRefusal::Loop,
        });
    }
    depth_check(chain, top_layer(chain) + 1)
}

/// 桥的一轮续到已有的链：Bot 已在链上就用自己的层（点名叫回沿用原链），
/// 不在就当新的一跳查层数。
pub(super) fn bridge_layer(chain: &StoredChain, bot: &BotRef) -> Result<u32, ChainError> {
    match layer_of(chain, bot) {
        Some(layer) => Ok(layer),
        None => depth_check(chain, top_layer(chain) + 1),
    }
}

fn depth_check(chain: &StoredChain, layer: u32) -> Result<u32, ChainError> {
    if layer > chain.limits.max_depth {
        return Err(ChainError::LimitReached {
            kind: ChainLimitKind::Depth,
        });
    }
    Ok(layer)
}
