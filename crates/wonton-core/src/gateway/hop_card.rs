//! 跳数到顶时问主人要不要继续（ADR-0045 的“频率和跳数”）：核心发一张确认卡片，“继续”清零放行，“先停着”什么都
//! 不做，房间等人说话或闲置够了再清零。卡片走总线的确认机制（ADR-0016），发起者记成被停下的那个 Bot 的插件。

use std::sync::Arc;

use wonton_protocol::{Answer, CardChoice, ConfirmationState};

use super::Gateway;
use super::send_limits::{HopLimitReached, SendLimiter};
use crate::l10n;
use crate::mcp::{Bus, Draft};
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;

/// 发卡片、等主人回答都在后台：Bot 的这次发送已经拿到 403，不等卡片。
pub(super) fn ask_to_continue(gateway: &Gateway, plugin: &PluginId, stopped: HopLimitReached) {
    let Some(mcp) = &gateway.mcp else {
        // 没有总线就没法问主人：如实记下，房间照样停着，等人说话再清零。
        tracing::error!(room = %stopped.room_id, "跳数到顶，但总线没启用，发不了“继续”卡片");
        return;
    };
    let bus = mcp.bus.clone();
    let limiter = gateway.send_limits.clone();
    let plugin = plugin.clone();
    tokio::spawn(ask(bus, limiter, plugin, stopped));
}

async fn ask(bus: Arc<Bus>, limiter: Arc<SendLimiter>, plugin: PluginId, stopped: HopLimitReached) {
    let langs = bus.owner_languages();
    let draft = draft(&stopped, limiter.hop_limit(), &langs);
    match bus.confirmations().ask(&plugin, draft).await {
        Ok(ticket) => {
            limiter.resume(&stopped.room_id);
            let message = l10n::text(&langs, "chat-hop-limit-resumed", &[]);
            bus.confirmations()
                .finish(ticket, ConfirmationState::Succeeded { message })
                .await;
        }
        Err(refusal) => tracing::info!(
            room = %stopped.room_id,
            reason = refusal.kind(),
            "主人没选继续，房间停着"
        ),
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
    }
}
