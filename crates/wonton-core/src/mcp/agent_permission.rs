//! Agent 权限确认（ADR-0035）：Agent 自己的工具要动手之前请主人确认。卡片走和 MCP 工具同一套（[`super::confirm`]），但
//! 核心不执行任何东西，只把主人选的那一项交回去；调用方断开就撤回，不像 MCP 的卡片那样留着等（ADR-0016）。

use std::collections::HashSet;
use std::sync::Arc;

use wonton_protocol::{
    AgentPermissionOption, AgentPermissionOutcome, AgentPermissionRequest, Answer,
    ConfirmationState,
};

use super::audience::Subject;
use super::audit::{AgentPermissionRecord, Decision};
use super::confirm::{CANCELLED, Refusal};
use super::{Bus, Caller, cards};
use crate::l10n;
use crate::plugin::languages::Languages;

/// 没能请主人确认的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPermissionError {
    InvalidInput(String),
    TooManyPending(String),
    Unavailable(String),
}

fn check(request: &AgentPermissionRequest) -> Result<(), AgentPermissionError> {
    let invalid = |message: &str| Err(AgentPermissionError::InvalidInput(message.to_owned()));
    if request.tool_call_id.is_empty() || request.title.is_empty() {
        return invalid("tool_call_id and title must not be empty");
    }
    if request.options.is_empty() {
        return invalid("options must not be empty");
    }
    let blank = |o: &AgentPermissionOption| o.choice_id.is_empty() || o.name.is_empty();
    if request.options.iter().any(blank) {
        return invalid("every option needs a non-empty choice_id and name");
    }
    let mut ids = HashSet::new();
    if !request.options.iter().all(|o| ids.insert(&o.choice_id)) {
        return invalid("choice_id is duplicated across options");
    }
    Ok(())
}

/// 发出去、还没有结果的一次确认。没等到结果就被丢下，就是调用方断开了：撤回卡片。
struct Asked {
    bus: Arc<Bus>,
    caller: String,
    card: String,
    /// 卡片认到的链（请求里带了 `session` 且会话在链上的时候）。
    chain: Option<String>,
    request: AgentPermissionRequest,
    /// 收件人的语言：卡片结束时的文字也用它。
    langs: Languages,
    settled: bool,
}

impl Asked {
    fn audit(&self, decision: Decision, option: Option<&AgentPermissionOption>) {
        self.bus.audit.record(&AgentPermissionRecord {
            caller: &self.caller,
            tool_call_id: &self.request.tool_call_id,
            title: &self.request.title,
            arguments: self.request.arguments.as_ref(),
            decision,
            choice_id: option.map(|o| o.choice_id.as_str()),
            kind: option.map(|o| o.kind),
        });
    }

    /// 主人按了 `choice_id` 的按钮。
    async fn answered(&mut self, choice_id: Option<&str>) -> AgentPermissionOutcome {
        self.settled = true;
        self.resolve_chain();
        if choice_id == Some(CANCELLED) {
            self.audit(Decision::Withdrawn, None);
            return AgentPermissionOutcome::Cancelled;
        }
        let option = self
            .request
            .options
            .iter()
            .find(|o| Some(o.choice_id.as_str()) == choice_id);
        let Some(option) = option else {
            self.audit(Decision::Rejected, None);
            self.bus.confirmations.bump();
            return AgentPermissionOutcome::Cancelled;
        };
        if option.kind.answer() == Answer::Approve {
            self.audit(Decision::Approved, Some(option));
            let state = ConfirmationState::Succeeded {
                message: l10n::text(
                    &self.langs,
                    "chat-agent-chose",
                    &[("choice", option.name.as_str().into())],
                ),
            };
            self.bus.confirmations.settle(&self.card, state).await;
        } else {
            self.audit(Decision::Rejected, Some(option));
            self.bus.confirmations.bump();
        }
        AgentPermissionOutcome::Selected {
            choice_id: option.choice_id.clone(),
        }
    }

    /// 卡片结束了（批、拒、取消、过期）：从链上摘掉，停链不再撤回它。
    fn resolve_chain(&self) {
        if self.chain.is_some()
            && let Some(chains) = self.bus.agent_chains()
        {
            let _ = chains.ledger().resolve_card(&self.card);
        }
    }
}

impl Drop for Asked {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        self.resolve_chain();
        self.audit(Decision::Withdrawn, None);
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let (bus, card) = (self.bus.clone(), std::mem::take(&mut self.card));
        let langs = self.langs.clone();
        runtime.spawn(async move {
            let state = ConfirmationState::Failed {
                message: l10n::text(&langs, "chat-agent-abandoned", &[]),
            };
            bus.confirmations.settle(&card, state).await;
        });
    }
}

impl Bus {
    /// 请主人确认，等到主人回答或者过期。调用方断开时丢下这个 future，卡片随之作废。
    pub async fn agent_permission(
        self: &Arc<Self>,
        caller: &Caller,
        request: AgentPermissionRequest,
    ) -> Result<AgentPermissionOutcome, AgentPermissionError> {
        check(&request)?;
        // 适配层知道 Agent 在哪个会话里干活，来源在请求体里；规则同 MCP 调用报的来源。
        let mut caller = caller.clone();
        caller.origin = request.origin.clone().and_then(super::audience::checked);
        let langs = self.card_languages(&caller, Subject::Tool);
        let draft = cards::agent_permission(&caller.name, &request, &langs);
        let audience = self.audience(&caller, Subject::Tool).await;
        let mut asked = Asked {
            bus: self.clone(),
            caller: caller.id.to_string(),
            card: String::new(),
            chain: None,
            request,
            langs,
            settled: true,
        };
        let (card, answered) = match self.confirmations.open_as(&caller.id, draft, audience) {
            Ok(opened) => opened,
            Err(refusal) => {
                asked.audit(Decision::Unconfirmed, None);
                return Err(refused(&refusal));
            }
        };
        asked.card.clone_from(&card.id);
        asked.settled = false;
        // 请求报了会话、会话在链上：卡片认到这条链，停链时撤回，状态查询里报 waiting_permission。
        asked.chain = card_chain(self, &asked.request).filter(|chain| {
            let bound = self
                .agent_chains()
                .is_some_and(|chains| chains.ledger().bind_card(chain, &card.id).is_ok());
            if !bound {
                tracing::warn!(card = %card.id, %chain, "权限卡片没能认到链上");
            }
            bound
        });
        match self.confirmations.deliver(&card, answered).await {
            Ok(chosen) => Ok(asked.answered(chosen.choice_id.as_deref()).await),
            Err(Refusal::Expired) => {
                asked.settled = true;
                asked.resolve_chain();
                asked.audit(Decision::Expired, None);
                Ok(AgentPermissionOutcome::Expired)
            }
            // 链被停：卡片已经被核心撤回了，回 cancelled 让 Agent 收手。
            Err(Refusal::Cancelled) => {
                asked.settled = true;
                asked.audit(Decision::Withdrawn, None);
                Ok(AgentPermissionOutcome::Cancelled)
            }
            Err(refusal) => {
                asked.settled = true;
                asked.audit(Decision::Unconfirmed, None);
                Err(refused(&refusal))
            }
        }
    }
}

/// 卡片该认到哪条链：请求里带了会话、会话登记在链上时是那条链。
fn card_chain(bus: &Bus, request: &AgentPermissionRequest) -> Option<String> {
    let session = request.session.as_deref()?;
    let chains = bus.agent_chains()?;
    chains
        .ledger()
        .session_chain(session)
        .map(|info| info.chain)
}

fn refused(refusal: &Refusal) -> AgentPermissionError {
    match refusal {
        Refusal::TooMany => AgentPermissionError::TooManyPending(refusal.message()),
        _ => AgentPermissionError::Unavailable(refusal.message()),
    }
}
