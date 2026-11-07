//! Agent 权限确认（ADR-0035）：Agent 自己的工具要动手之前请主人确认。卡片走和 MCP 工具同一套（[`super::confirm`]），但
//! 核心不执行任何东西，只把主人选的那一项交回去；调用方断开就撤回，不像 MCP 的卡片那样留着等（ADR-0016）。

use std::collections::HashSet;
use std::sync::Arc;

use wonton_protocol::{
    AgentPermissionOption, AgentPermissionOutcome, AgentPermissionRequest, Answer,
    ConfirmationState,
};

use super::audit::{AgentPermissionRecord, Decision};
use super::confirm::Refusal;
use super::{Bus, Caller, cards};
use crate::l10n;

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
    request: AgentPermissionRequest,
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
        let option = self
            .request
            .options
            .iter()
            .find(|o| Some(o.choice_id.as_str()) == choice_id);
        let Some(option) = option else {
            self.audit(Decision::Rejected, None);
            self.bus.confirmations.publish(&self.card).await;
            return AgentPermissionOutcome::Cancelled;
        };
        if option.kind.answer() == Answer::Approve {
            self.audit(Decision::Approved, Some(option));
            let langs = self.bus.plugins.owner_languages();
            let state = ConfirmationState::Succeeded {
                message: l10n::text(
                    &langs,
                    "chat-agent-chose",
                    &[("choice", option.name.as_str().into())],
                ),
            };
            self.bus.confirmations.settle(&self.card, state).await;
        } else {
            self.audit(Decision::Rejected, Some(option));
            self.bus.confirmations.publish(&self.card).await;
        }
        AgentPermissionOutcome::Selected {
            choice_id: option.choice_id.clone(),
        }
    }
}

impl Drop for Asked {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        self.audit(Decision::Withdrawn, None);
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let (bus, card) = (self.bus.clone(), std::mem::take(&mut self.card));
        runtime.spawn(async move {
            let langs = bus.plugins.owner_languages();
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
        let langs = self.plugins.owner_languages();
        let draft = cards::agent_permission(&caller.name, &request, &langs);
        let mut asked = Asked {
            bus: self.clone(),
            caller: caller.id.to_string(),
            card: String::new(),
            request,
            settled: true,
        };
        let (card, answered) = match self.confirmations.open(&caller.id, draft) {
            Ok(opened) => opened,
            Err(refusal) => {
                asked.audit(Decision::Unconfirmed, None);
                return Err(refused(&refusal));
            }
        };
        asked.card.clone_from(&card.id);
        asked.settled = false;
        match self.confirmations.deliver(&card, answered).await {
            Ok(chosen) => Ok(asked.answered(chosen.choice_id.as_deref()).await),
            Err(Refusal::Expired) => {
                asked.settled = true;
                asked.audit(Decision::Expired, None);
                Ok(AgentPermissionOutcome::Expired)
            }
            Err(refusal) => {
                asked.settled = true;
                asked.audit(Decision::Unconfirmed, None);
                Err(refused(&refusal))
            }
        }
    }
}

fn refused(refusal: &Refusal) -> AgentPermissionError {
    match refusal {
        Refusal::TooMany => AgentPermissionError::TooManyPending(refusal.message()),
        _ => AgentPermissionError::Unavailable(refusal.message()),
    }
}
