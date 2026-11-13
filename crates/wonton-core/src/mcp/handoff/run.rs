use std::sync::Arc;

use tokio::sync::mpsc;
use wonton_protocol::{
    ACP_CHAIN_META_KEY, ChainCarrier, ConfirmationState, HandOffResult, HandOffSession,
    HandOffState,
};

use super::{Context, OpenResult, Operation, chain_error, client, consent};
use crate::mcp::{Bus, Caller, ToolFailure};

pub(super) async fn start(
    bus: &Arc<Bus>,
    caller: &Caller,
    context: Context,
    operation: &Arc<Operation>,
) -> Result<(), ToolFailure> {
    let ledger = context.chains.ledger();
    let layer = ledger
        .preview_attach_from_session(&context.parent, &context.request.target_bot)
        .map_err(chain_error)?;
    if !context
        .gateway
        .bot_listens_to(&context.request.target_bot, &context.user)
    {
        return Err(ToolFailure::new(
            "permission_denied",
            "目标 Bot 不听这条链代表的人",
        ));
    }
    let langs = bus.owner_languages();
    let route = bus
        .plugins
        .handoff_route(&caller.id, &context.request.target_bot, &langs)
        .await
        .map_err(|error| match error {
            crate::plugin::dependencies::CapabilityAccessError::Denied => {
                ToolFailure::new("permission_denied", "目标不提供已声明的 agent_acp 能力")
            }
            crate::plugin::dependencies::CapabilityAccessError::Unavailable(message) => {
                ToolFailure::new("dependency_stopped", message)
            }
        })?;
    let card = consent::ask(bus, caller, &context).await?;
    let outcome = connect(bus, caller, &context, layer, route, operation).await;
    if let Some(card) = card {
        let state = match &outcome {
            Ok(()) => ConfirmationState::Succeeded {
                message: crate::l10n::text(&langs, "chat-handoff-started", &[]),
            },
            Err(error) => ConfirmationState::Failed {
                message: error.message.clone(),
            },
        };
        bus.confirmations.settle(&card, state).await;
    }
    outcome
}

async fn connect(
    bus: &Arc<Bus>,
    caller: &Caller,
    context: &Context,
    layer: u32,
    route: crate::plugin::dependencies::CapabilityRoute,
    operation: &Arc<Operation>,
) -> Result<(), ToolFailure> {
    let mut connection = crate::gateway::agent_acp::connect_handoff(
        &context.gateway,
        &caller.id,
        &context.request.target_bot,
        &context.user,
        &context.chain,
        route,
    )
    .await?;
    let meta = serde_json::json!({
        "app.wonton/task": {"name": context.request.message, "room": context.origin.room_id, "thread": context.origin.thread_id},
        (ACP_CHAIN_META_KEY): ChainCarrier { id: context.chain.clone(), depth: layer },
    }).as_object().expect("元数据是对象").clone();
    let (events, mut received) = mpsc::unbounded_channel();
    let message = context.request.message.clone();
    let task = tokio::spawn(client::run(
        connection.stream,
        meta,
        message,
        events,
        connection.cancel,
    ));
    let mut session = None;
    while let Some(event) = received.recv().await {
        match event {
            client::Event::Opened {
                session: opened,
                proceed,
            } => {
                let registered =
                    register(bus, context, layer, &opened, &mut connection.registration);
                let allowed = registered.is_ok();
                let _ = proceed.send(allowed);
                if let Err(error) = registered {
                    // The SDK drains the cancellation before closing its transport.
                    let _ = task.await;
                    return Err(error);
                }
                session = Some(opened);
            }
            client::Event::PromptSent => {
                let opened = session.as_ref().expect("发送前已登记会话");
                operation.finish(Ok(HandOffResult {
                    session: opened.clone(),
                    chain: context.chain.clone(),
                }));
                let bus = bus.clone();
                let chains = context.chains.clone();
                let opened = opened.clone();
                tokio::spawn(async move {
                    observe(&bus, &opened, &mut received).await;
                    if !matches!(task.await, Ok(Ok(()))) {
                        bus.handoffs.ended(&opened, None);
                    }
                    chains.ledger().end_session(&opened);
                    drop(connection.registration);
                });
                return Ok(());
            }
            client::Event::Text(_) | client::Event::Ended { .. } => {
                return Err(ToolFailure::new(
                    "upstream_error",
                    "目标在收到首轮前发回了轮次结果",
                ));
            }
        }
    }
    let detail = match task.await {
        Ok(Err(error)) => error,
        Ok(Ok(())) => "目标没有接收首轮".into(),
        Err(error) => error.to_string(),
    };
    if let Some(session) = session {
        bus.handoffs.ended(&session, None);
        context.chains.ledger().end_session(&session);
    }
    Err(ToolFailure::new("upstream_error", detail))
}

fn register(
    bus: &Bus,
    context: &Context,
    layer: u32,
    session: &str,
    registration: &mut crate::gateway::agent_acp::Registration,
) -> OpenResult {
    if !registration.bind_session(session) {
        return Err(ToolFailure::new(
            "upstream_error",
            "目标返回了仍在使用的会话 ID",
        ));
    }
    let ledger = context.chains.ledger();
    let actual = ledger
        .register_session_from_parent(
            &context.parent,
            session,
            context.request.target_bot.clone(),
            crate::agent_chain::ChainSessionSource::HandOff,
        )
        .map_err(chain_error)?;
    if actual != layer
        || ledger
            .session_chain(session)
            .is_none_or(|info| info.chain != context.chain)
    {
        ledger.end_session(session);
        return Err(ToolFailure::new(
            "invalid_input",
            "建会话期间调用路径变了，没有派发首轮",
        ));
    }
    if let Err(error) = ledger.begin_round(session) {
        ledger.end_session(session);
        return Err(chain_error(error));
    }
    let now = crate::peers::now_ms();
    bus.handoffs.opened(
        context.from.clone(),
        context.origin.clone(),
        HandOffSession {
            session: session.to_owned(),
            bot: context.request.target_bot.clone(),
            chain: context.chain.clone(),
            status: HandOffState::Running,
            stop_reason: None,
            started_at: now,
            last_activity_at: now,
            summary: String::new(),
        },
    );
    Ok(HandOffResult {
        session: session.to_owned(),
        chain: context.chain.clone(),
    })
}

async fn observe(bus: &Bus, session: &str, received: &mut mpsc::UnboundedReceiver<client::Event>) {
    while let Some(event) = received.recv().await {
        match event {
            client::Event::Text(text) => bus.handoffs.text(session, &text),
            client::Event::Ended { stop_reason } => {
                bus.handoffs.ended(session, Some(stop_reason));
                return;
            }
            client::Event::Opened { .. } | client::Event::PromptSent => {
                bus.handoffs.ended(session, None);
                return;
            }
        }
    }
    bus.handoffs.ended(session, None);
}
