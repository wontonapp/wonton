//! 跳数到顶时问 Bot 的归属人要不要继续（ADR-0045 的“频率和跳数”）。
//!
//! 每个归属人一张卡片：私人 Bot 发给它的归属人，共享 Bot 发给主人，两个 Bot 归同一个人时一张。卡片的发起者是这个
//! 归属人的 Bot，来源会话是被停下的房间。所有卡片都选了“继续”才清零放行：任何一方选“先停着”或没回答，房间就停着，
//! 等人说话或闲置够了再清零。只要一方同意就放行，等于替另一方的 Bot 决定接着说。卡片走总线的确认机制（ADR-0016）。

use std::collections::BTreeMap;
use std::sync::Arc;

use futures_util::future::join_all;
use wonton_protocol::{
    Answer, BotRef, CardChoice, ConfirmationOrigin, ConfirmationRequester, ConfirmationState,
};

use super::{HopLimitReached, SendLimiter};
use crate::gateway::Gateway;
use crate::l10n;
use crate::mcp::{Audience, Bus, Draft};
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, PluginManager};

/// 一张卡片：发给谁（`None` 是主人），用哪个 Bot 的名义。
struct Ask {
    recipient: Option<String>,
    plugin: PluginId,
    bot: String,
    user_id: String,
}

/// 发卡片、等回答都在后台：Bot 的这次发送已经拿到 403，不等卡片。
pub(in crate::gateway) fn ask_to_continue(gateway: &Gateway, stopped: HopLimitReached) {
    let Some(mcp) = &gateway.mcp else {
        // 没有总线就没法问归属人：如实记下，房间照样停着，等人说话再清零。
        tracing::error!(room = %stopped.room_id, "跳数到顶，但总线没启用，发不了“继续”卡片");
        return;
    };
    let asks = asks(&gateway.plugins, gateway.owner.username(), &stopped);
    if asks.is_empty() {
        tracing::error!(room = %stopped.room_id, bots = ?stopped.bot_user_ids, "跳数到顶，但认不出 Bot 的归属人，发不了卡片");
        return;
    }
    let bus = mcp.bus.clone();
    let limiter = gateway.send_limits.clone();
    tokio::spawn(ask_all(bus, limiter, asks, stopped));
}

/// 按归属人分卡片：同一个人的 Bot 只占一张，用排在前面的那个 Bot 的名义。
fn asks(plugins: &PluginManager, owner: &str, stopped: &HopLimitReached) -> Vec<Ask> {
    let mut by_keeper: BTreeMap<Option<String>, Ask> = BTreeMap::new();
    for user_id in &stopped.bot_user_ids {
        let Some((plugin, bot, keeper)) = plugins.bot_by_user(user_id) else {
            tracing::warn!(bot = %user_id, "跳数到顶的 Bot 不在 Bot 记录里，不给它发卡片");
            continue;
        };
        let recipient = (keeper != owner).then_some(keeper);
        by_keeper.entry(recipient.clone()).or_insert(Ask {
            recipient,
            plugin,
            bot,
            user_id: user_id.clone(),
        });
    }
    by_keeper.into_values().collect()
}

async fn ask_all(
    bus: Arc<Bus>,
    limiter: Arc<SendLimiter>,
    asks: Vec<Ask>,
    stopped: HopLimitReached,
) {
    let hops = limiter.hop_limit();
    let answers = join_all(asks.into_iter().map(|ask| {
        let bus = bus.clone();
        // 卡片文字按收件人的语言（ADR-0043）：主人的用主人的，家人的用他最近一次请求的。
        let langs = bus.languages_for(ask.recipient.as_deref());
        let draft = draft(&stopped, hops, &langs);
        let audience = Audience {
            recipients: crate::mcp::Recipients::Keeper(ask.recipient.clone()),
            requester: Some(ConfirmationRequester::Bot {
                bot: BotRef {
                    plugin_id: ask.plugin.to_string(),
                    bot: ask.bot.clone(),
                },
                user_id: ask.user_id.clone(),
            }),
            origin: Some(ConfirmationOrigin {
                room_id: stopped.room_id.clone(),
                thread_id: None,
            }),
            category: None,
        };
        async move {
            let answer = bus
                .confirmations()
                .ask_as(&ask.plugin, draft, audience)
                .await;
            (langs, answer)
        }
    }))
    .await;
    let all_continue = answers.iter().all(|(_, answer)| answer.is_ok());
    if all_continue {
        limiter.resume(&stopped.room_id);
    }
    for (langs, answer) in answers {
        match answer {
            Ok(ticket) => {
                let state = if all_continue {
                    ConfirmationState::Succeeded {
                        message: l10n::text(&langs, "chat-hop-limit-resumed", &[]),
                    }
                } else {
                    ConfirmationState::Failed {
                        message: l10n::text(&langs, "chat-hop-limit-kept-by-other", &[]),
                    }
                };
                bus.confirmations().finish(ticket, state).await;
            }
            Err(refusal) => tracing::info!(
                room = %stopped.room_id,
                reason = refusal.kind(),
                "归属人没选继续，房间停着"
            ),
        }
    }
}

fn draft(stopped: &HopLimitReached, hops: u32, langs: &Languages) -> Draft {
    Draft {
        title: l10n::text(
            langs,
            "chat-card-hop-limit-title",
            &[
                ("bots", l10n::list(langs, &stopped.bot_user_ids).into()),
                ("hops", hops.into()),
            ],
        ),
        lines: Vec::new(),
        choices: vec![
            CardChoice {
                answer: Answer::Approve,
                label: l10n::text(langs, "chat-choice-hop-limit-continue", &[]),
                choice_id: None,
            },
            CardChoice {
                answer: Answer::Reject,
                label: l10n::text(langs, "chat-choice-hop-limit-stay", &[]),
                choice_id: None,
            },
        ],
        scope: None,
        plugin_id: None,
        secret_inputs: Vec::new(),
        settings: None,
        category: None,
    }
}
