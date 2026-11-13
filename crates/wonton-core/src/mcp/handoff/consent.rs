use std::sync::Arc;

use wonton_protocol::{
    Answer, CardChoice, ConfirmationCategory, ConfirmationRequester, ConfirmationState,
};

use super::super::{Bus, Caller, ToolFailure, cards};
use crate::l10n;
use crate::plugin::PluginId;

pub(super) async fn ask(
    bus: &Arc<Bus>,
    caller: &Caller,
    context: &super::Context,
) -> Result<Option<String>, ToolFailure> {
    let target = &context.request.target_bot;
    let id = PluginId::parse(&target.plugin_id)
        .map_err(|error| ToolFailure::new("invalid_input", error.to_string()))?;
    let record = bus
        .plugins
        .store()
        .bots()
        .entry(&id, &target.bot)
        .map_err(|error| ToolFailure::new("storage", error.to_string()))?
        .ok_or_else(|| ToolFailure::new("permission_denied", "目标 Bot 没有本机记录"))?;
    let keeper = bus
        .plugins
        .bot_by_user(&record.account.user_id)
        .map(|(_, _, keeper)| keeper)
        .ok_or_else(|| ToolFailure::new("permission_denied", "认不出目标 Bot 的归属人"))?;
    let own = bus
        .people()
        .map_err(|error| ToolFailure::new("service_unavailable", error))?;
    let keeper_id = format!("@{keeper}:{}", own.server_name);
    if context.user == keeper_id
        && bus
            .plugins
            .handoff_confirmed(&context.from, target)
            .map_err(|error| ToolFailure::new("storage", error.to_string()))?
    {
        return Ok(None);
    }
    let mut target_caller = Caller::for_bot(id, None, target.bot.clone(), record.keeper);
    target_caller.origin = Some(context.origin.clone());
    let mut audience = bus
        .audience(&target_caller, crate::mcp::audience::Subject::Tool)
        .await;
    let from_account = bus
        .plugins
        .bot_account(&caller.id, &context.from.bot)
        .ok_or_else(|| ToolFailure::new("permission_denied", "发起 Bot 没有本机记录"))?;
    audience.requester = Some(ConfirmationRequester::Bot {
        bot: context.from.clone(),
        user_id: from_account.user_id,
    });
    let langs = bus.languages_for(audience.recipient.as_deref());
    let draft = draft(bus, caller, context, &langs);
    let (card, answered) = bus
        .confirmations
        .open_as(&caller.id, draft, audience)
        .map_err(|refusal| ToolFailure::new(refusal.kind(), refusal.message()))?;
    if let Err(error) = context.chains.ledger().bind_card(&context.chain, &card.id) {
        bus.confirmations.cancel(&card.id, &error.to_string());
        return Err(super::chain_error(error));
    }
    if context
        .chains
        .ledger()
        .row(&context.chain)
        .is_some_and(|row| row.status != wonton_protocol::AgentChainStatus::Running)
    {
        bus.confirmations.cancel(&card.id, "调用链已停止或用到上限");
    }
    let chosen = bus.confirmations.deliver(&card, answered).await;
    let _ = context.chains.ledger().resolve_card(&card.id);
    if context
        .chains
        .ledger()
        .row(&context.chain)
        .is_some_and(|row| row.status == wonton_protocol::AgentChainStatus::Stopped)
    {
        return Err(ToolFailure::new("stopped", "这条调用链已停下"));
    }
    let chosen = chosen.map_err(|refusal| ToolFailure::new(refusal.kind(), refusal.message()))?;
    if chosen.answer != Answer::Approve {
        return Err(ToolFailure::new("owner_rejected", "归属人拒绝了，没有交接"));
    }
    if let Err(error) = bus.plugins.confirm_handoff(&context.from, target) {
        bus.confirmations
            .settle(
                &card.id,
                ConfirmationState::Failed {
                    message: error.to_string(),
                },
            )
            .await;
        return Err(ToolFailure::new("storage", error.to_string()));
    }
    Ok(Some(card.id))
}

fn draft(
    bus: &Bus,
    caller: &Caller,
    context: &super::Context,
    langs: &crate::plugin::languages::Languages,
) -> cards::Draft {
    let target = &context.request.target_bot;
    let title = bus
        .plugins
        .bot_display_name(target)
        .unwrap_or_else(|| target.bot.clone());
    cards::Draft {
        title: l10n::text(
            langs,
            "chat-handoff-title",
            &[
                ("from", caller.name.as_str().into()),
                ("bot", title.as_str().into()),
            ],
        ),
        lines: vec![
            cards::line(&context.request.message),
            cards::emphasized(l10n::text(langs, "chat-handoff-risk", &[])),
        ],
        choices: vec![
            CardChoice {
                answer: Answer::Approve,
                label: l10n::text(langs, "chat-choice-allow", &[]),
                choice_id: None,
            },
            CardChoice {
                answer: Answer::Reject,
                label: l10n::text(langs, "chat-choice-deny", &[]),
                choice_id: None,
            },
        ],
        scope: None,
        plugin_id: Some(target.plugin_id.clone()),
        secret_inputs: Vec::new(),
        settings: None,
        category: Some(ConfirmationCategory::HandOffConsent {
            target_bot: target.clone(),
            from: context.from.clone(),
        }),
    }
}
