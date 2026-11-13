//! `wonton__send_message`（ADR-0061 第 9 节）：Bot 替归属人给家人、好友发消息。收件人先过
//! 名单（[`Members`]、[`crate::friends::Friends`]），陌生人直接拒绝，不发卡片。批准后不投递，
//! 排进这个 Bot 的出箱（[`OutboxKind::Message`]）就返回项 ID；桥取走后发进和收件人的私聊。
//!
//! 放行时段按收件人放宽（ADR-0024），只在链的发起人是这个 Bot 的归属人时生效（ADR-0083
//! 第 9 条）：发起人认不清、或不是归属人（如好友在群里让 Bot 发），每条都问。

use std::sync::Arc;

use rmcp::model::{CallToolResult, JsonObject, Tool};
use serde_json::{Value, json};
use wonton_l10n::Arg;
use wonton_protocol::{
    Answer, BotRef, CardChoice, ConfirmationCategory, MemberStatus, OutboxKind, SendMessageRequest,
};

use super::Refusal;
use super::audience::Subject;
use super::audit::{Basis, Decision, Grounds};
use super::cards::{Draft, line};
use super::{Bus, Caller, Outcome, ToolFailure, core_tools, with_confirmation, with_failure};
use crate::friends::FriendStatus;
use crate::plugin::languages::Languages;

pub(super) const SEND_MESSAGE: &str = "send_message";

pub(super) fn definition() -> Tool {
    core_tools::tool(
        SEND_MESSAGE,
        "替主人的家人或好友发一条消息：把正文发进这个 Bot 和收件人的私聊。收件人用用户名，\
         只能是这个 Bot 主人的家人或好友；每条消息都会先请归属人在 Wonton App 里确认，\
         批准后排入发送队列就返回，投递在后台进行。调用前先告诉主人你要给谁发什么。",
        json!({
            "type": "object",
            "properties": {
                "recipient": {"type": "string", "description": "收件人的用户名"},
                "body": {"type": "string", "description": "要发的正文"},
            },
            "required": ["recipient", "body"],
        }),
        false,
    )
}

pub(super) async fn call(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    args: JsonObject,
    activity: crate::bot_activity::Guard,
) -> CallToolResult {
    let name_owned = name.to_owned();
    let args = Value::Object(args);
    let request: SendMessageRequest = match serde_json::from_value(args.clone()) {
        Ok(request) => request,
        Err(e) => return bus.deny(caller, name, &args, &invalid(format!("参数不对：{e}"))),
    };
    let Some(identity) = caller.bot.as_ref() else {
        return denied_no_bot(bus, caller, name, &args);
    };
    let Some(account) = bus.plugins.bot_account(&caller.id, &identity.bot) else {
        return bus.deny(
            caller,
            name,
            &args,
            &ToolFailure::new("permission_denied", "这个 Bot 没有登记 Matrix 账号"),
        );
    };
    let Some((_, _, keeper)) = bus.plugins.bot_by_user(&account.user_id) else {
        return bus.deny(
            caller,
            name,
            &args,
            &ToolFailure::new("permission_denied", "认不出这个 Bot 的归属人"),
        );
    };
    let Some(display_name) = resolve_recipient(bus, &request.recipient) else {
        return bus.deny(
            caller,
            name,
            &args,
            &ToolFailure::new(
                "permission_denied",
                "收件人不是这个 Bot 归属人的家人或好友，不能发",
            ),
        );
    };
    // 放行时段按收件人放宽，前提是链的发起人就是这个 Bot 的归属人；认不清就不放宽。
    let initiator_is_keeper = caller
        .chain
        .as_ref()
        .and_then(|chain| {
            bus.agent_chains()
                .and_then(|chains| chains.ledger().row(chain))
        })
        .and_then(|row| initiator_username(bus, &row.user))
        .is_some_and(|user| user == keeper);
    if initiator_is_keeper
        && let Some(window) = bus
            .confirmations()
            .allow_in_recipient_window(&caller.id, &request.recipient)
    {
        let langs = bus.languages_for(None);
        return relaxed(
            bus,
            caller,
            name,
            &args,
            &request,
            window,
            crate::l10n::text(
                &langs,
                "chat-send-message-queued",
                &[("bot", Arg::from(caller.name.as_str()))],
            ),
        );
    }
    drop(activity);
    let bus = bus.clone();
    let caller = caller.clone();
    let task = tokio::spawn(async move {
        ask_and_enqueue(bus, caller, name_owned, args, request, display_name).await
    });
    task.await
        .unwrap_or_else(|e| with_failure(&format!("确认流程意外中止：{e}"), "internal_error"))
}

fn denied_no_bot(bus: &Bus, caller: &Caller, name: &str, args: &Value) -> CallToolResult {
    bus.deny(
        caller,
        name,
        args,
        &ToolFailure::new(
            "permission_denied",
            "wonton__send_message 只发给有 Bot 的调用方",
        ),
    )
}

/// 主人先前放行了这个收件人：不发卡片直接入队，结果照实带 `app.wonton/confirmation: window`。
fn relaxed(
    bus: &Arc<Bus>,
    caller: &Caller,
    name: &str,
    args: &Value,
    request: &SendMessageRequest,
    window: String,
    queued_message: String,
) -> CallToolResult {
    match enqueue(bus, caller, request) {
        Ok(id) => {
            let outcome = queued(&id, queued_message);
            bus.record(
                caller,
                name,
                args,
                Decision::Allowed,
                Some(Grounds::from(Basis::Window(window))),
                &outcome,
            );
            with_confirmation(outcome.result, "window")
        }
        Err(failure) => bus.deny(caller, name, args, &failure),
    }
}

/// 发卡片、等回答，批准了入队。整个过程在后台任务里：Agent 等不及断开了，确认照样有效
/// 到过期，主人批准后照样入队，结果在卡片上（ADR-0016）。
async fn ask_and_enqueue(
    bus: Arc<Bus>,
    caller: Caller,
    name: String,
    args: Value,
    request: SendMessageRequest,
    display_name: String,
) -> CallToolResult {
    let langs = bus.card_languages(&caller, Subject::Tool);
    let draft = draft(&bus, &caller, &request, &display_name, &langs);
    let mut audience = bus.audience(&caller, Subject::Tool).await;
    audience.category = Some(ConfirmationCategory::SendMessage {
        recipient: request.recipient.clone(),
        body: request.body.clone(),
    });
    let (card, answered) = match bus.confirmations().open_as(&caller.id, draft, audience) {
        Ok(opened) => opened,
        Err(refusal) => return bus.refused(&caller, &name, &args, &refusal, None),
    };
    // 卡片认到这条链：链被停时随停撤回（等的人拿到 cancelled，不入队）。
    if let Some(chain) = &caller.chain
        && let Some(chains) = bus.agent_chains()
        && let Err(error) = chains.ledger().bind_card(chain, &card.id)
    {
        tracing::warn!(%error, card = %card.id, chain, "发消息卡片没能认到链上");
    }
    match bus.confirmations().deliver(&card, answered).await {
        Ok(chosen)
            if matches!(
                chosen.answer,
                Answer::Approve | Answer::ApproveWider | Answer::AllowWindow
            ) =>
        {
            // 卡片批了；入队失败时批准的事实照记，结果说明没排上。
            let outcome = match enqueue(&bus, &caller, &request) {
                Ok(id) => queued(
                    &id,
                    crate::l10n::text(
                        &langs,
                        "chat-send-message-queued",
                        &[("bot", Arg::from(caller.name.as_str()))],
                    ),
                ),
                Err(failure) => Outcome::failed(failure.kind, failure.message),
            };
            bus.record(
                &caller,
                &name,
                &args,
                Decision::Approved,
                Some(Grounds::from(Basis::Card)),
                &outcome,
            );
            bus.confirmations()
                .settle(&card.id, outcome.state.clone())
                .await;
            with_confirmation(outcome.result, "card")
        }
        // 拒绝时卡片状态已经发布过（[`Confirmations::deliver`]）。
        Ok(_) => bus.refused(&caller, &name, &args, &Refusal::Rejected, None),
        Err(refusal) => bus.refused(&caller, &name, &args, &refusal, None),
    }
}

/// 收件人是不是这个 Bot 归属人的家人或好友：先查家人（移除的不算），再查好友（删掉的不算）。
/// 返回卡片上显示的名字（资料里的显示名，没有就用户名）。
fn resolve_recipient(bus: &Bus, recipient: &str) -> Option<String> {
    if let Ok(people) = bus.people()
        && let Some(record) = people.members.get(recipient)
        && record.status != MemberStatus::Removed
    {
        return Some(display_name_of(Some(&record.display_name), recipient));
    }
    bus.friends()
        .and_then(|friends| friends.find_username(recipient))
        .filter(|record| record.status == FriendStatus::Active)
        .map(|record| display_name_of(record.display_name.as_deref(), recipient))
}

/// 卡片上显示的名字：资料里的显示名，没填就用户名。
/// 链发起人的本机用户名：`@名:本服务器` 才认得出来，别家的认不出（就不放宽）。
fn initiator_username(bus: &Bus, user_id: &str) -> Option<String> {
    let server = bus.people().ok()?.server_name.as_str();
    let rest = user_id.strip_prefix('@')?;
    let (name, host) = rest.split_once(':')?;
    (host == server).then(|| name.to_owned())
}

fn display_name_of(display_name: Option<&str>, fallback: &str) -> String {
    display_name
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

/// 批准了：排进这个 Bot 的出箱，不等投递。
fn enqueue(
    bus: &Arc<Bus>,
    caller: &Caller,
    request: &SendMessageRequest,
) -> Result<String, ToolFailure> {
    let Some(chains) = bus.agent_chains() else {
        return Err(ToolFailure::new(
            "source_unavailable",
            "调用链没接上，出箱不可用",
        ));
    };
    let bot = BotRef {
        plugin_id: caller.id.to_string(),
        bot: caller.bot.as_ref().expect("入口已认过 Bot").bot.clone(),
    };
    chains
        .outbox()
        .push(
            &bot,
            OutboxKind::Message {
                recipient: request.recipient.clone(),
                body: request.body.clone(),
            },
        )
        .map_err(|error| ToolFailure::new("storage", error.to_string()))
}

/// 给 Agent 的结果：出箱项 ID，投递结果不在工具调用里等。
fn queued(id: &str, message: String) -> Outcome {
    Outcome::ok(format!(r#"{{"outbox_item": "{id}"}}"#), message)
}

fn invalid(message: String) -> ToolFailure {
    ToolFailure::new("invalid_input", message)
}

fn draft(
    bus: &Bus,
    caller: &Caller,
    request: &SendMessageRequest,
    display_name: &str,
    langs: &Languages,
) -> Draft {
    Draft {
        title: crate::l10n::text(
            langs,
            "chat-send-message-title",
            &[("recipient", Arg::from(display_name))],
        ),
        lines: vec![line(&request.body)],
        choices: choices(bus, langs),
        // 不进插件的放行时段：发消息的时段按收件人开（[`super::confirm::Confirmations`]）。
        scope: None,
        plugin_id: Some(caller.id.to_string()),
        secret_inputs: Vec::new(),
        settings: None,
    }
}

fn choices(bus: &Bus, langs: &Languages) -> Vec<CardChoice> {
    let window = bus.confirmations().window_length();
    let window_label = crate::l10n::text(
        langs,
        if window.as_secs().is_multiple_of(60) {
            "chat-card-window-minutes"
        } else {
            "chat-card-window-seconds"
        },
        &[if window.as_secs().is_multiple_of(60) {
            ("minutes", Arg::from(window.as_secs() / 60))
        } else {
            ("secs", Arg::from(window.as_secs()))
        }],
    );
    vec![
        CardChoice {
            answer: Answer::Approve,
            label: crate::l10n::text(langs, "chat-card-approve", &[]),
            choice_id: None,
        },
        CardChoice {
            answer: Answer::AllowWindow,
            label: window_label,
            choice_id: None,
        },
        CardChoice {
            answer: Answer::Reject,
            label: crate::l10n::text(langs, "chat-card-reject", &[]),
            choice_id: None,
        },
    ]
}
