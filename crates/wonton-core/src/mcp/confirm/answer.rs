//! 主人的回答：校验按钮和密钥，把回答交给等待方，点“放行”时开时段。

use std::time::SystemTime;

use wonton_protocol::{Answer, CardChoice, ConfirmationState, SecretValues};

use super::types::AnswerError;
use super::{Chosen, Confirmations, now_ms};
use crate::plugin::languages::Languages;

impl Confirmations {
    /// 主人的回答。确认后卡片变成“正在执行”，拒绝后变成“已拒绝”。
    pub fn answer(
        &self,
        id: &str,
        answer: Answer,
        choice_id: Option<&str>,
    ) -> Result<ConfirmationState, AnswerError> {
        self.answer_with_secrets(
            id,
            answer,
            choice_id,
            SecretValues::new(),
            Default::default(),
            Vec::new(),
            &Languages::from_header(None),
        )
    }

    /// 带密钥的回答（ADR-0064）。
    #[allow(clippy::too_many_arguments)] // Owner answer values are forwarded together without duplicating form state.
    pub fn answer_with_secrets(
        &self,
        id: &str,
        answer: Answer,
        choice_id: Option<&str>,
        secrets: SecretValues,
        settings: wonton_protocol::SettingValues,
        devices: Vec<String>,
        langs: &Languages,
    ) -> Result<ConfirmationState, AnswerError> {
        self.answer_with_secrets_by(
            id, answer, choice_id, secrets, settings, devices, langs, None,
        )
    }

    /// Gateway-authenticated answerer, recorded atomically with the winning answer.
    #[allow(clippy::too_many_arguments)]
    pub fn answer_with_secrets_by(
        &self,
        id: &str,
        answer: Answer,
        choice_id: Option<&str>,
        secrets: SecretValues,
        settings: wonton_protocol::SettingValues,
        devices: Vec<String>,
        langs: &Languages,
        answered_by: Option<&str>,
    ) -> Result<ConfirmationState, AnswerError> {
        let state = match answer {
            Answer::Approve | Answer::ApproveWider | Answer::AllowWindow => {
                ConfirmationState::Running
            }
            Answer::Reject => ConfirmationState::Rejected,
            Answer::Unknown => return Err(AnswerError::UnknownAnswer),
        };
        let now = now_ms(SystemTime::now());
        let mut table = self.lock();
        let slot = table.slots.get_mut(id).ok_or(AnswerError::NotFound)?;
        // 卡片上没有这个按钮，就不是对这张卡片的回答。
        let on_card = |c: &CardChoice| c.answer == answer && c.choice_id.as_deref() == choice_id;
        if !slot.card.choices.iter().any(on_card) {
            return Err(AnswerError::UnknownAnswer);
        }
        if slot.card.state == ConfirmationState::Expired || now >= slot.card.expires_at_ms {
            return Err(AnswerError::Expired);
        }
        // 先查密钥：不对时卡片照常等回答。
        let secrets = if slot.secret_inputs.is_empty() {
            if !secrets.is_empty() {
                return Err(AnswerError::Secrets(crate::l10n::text(
                    langs,
                    "chat-card-secrets-not-used",
                    &[],
                )));
            }
            secrets
        } else {
            if let Some(name) = secrets
                .keys()
                .find(|name| !slot.secret_inputs.iter().any(|i| &i.name == *name))
            {
                return Err(AnswerError::Secrets(crate::l10n::text(
                    langs,
                    "plugin-secret-unknown",
                    &[("names", crate::l10n::list(langs, &[name.as_str()]).into())],
                )));
            }
            // 必填项没填也能回答：插件照样装，状态是“待配置”（ADR-0064）。
            secrets
        };
        let Some(waiter) = slot.waiter.take() else {
            return Err(AnswerError::AlreadyAnswered);
        };
        // 等待的一方已经不在（只会在核心退出时发生）：当作过期。
        let chosen = Chosen {
            answer,
            choice_id: choice_id.map(str::to_owned),
            secrets,
            settings,
            devices,
        };
        if waiter.send(chosen).is_err() {
            slot.card.state = ConfirmationState::Expired;
            slot.ended = Some(now);
            self.activity_changed();
            self.bump();
            return Err(AnswerError::Expired);
        }
        slot.card.state = state.clone();
        slot.card.answered_by = answered_by.map(str::to_owned);
        slot.ended = Some(now);
        if answer == Answer::AllowWindow {
            // 发消息的卡片按收件人放宽，其余按插件。
            let send_message = matches!(
                slot.card.category,
                Some(wonton_protocol::ConfirmationCategory::SendMessage { .. })
            );
            if send_message {
                self.open_recipient_window(&mut table, id, now);
            } else {
                self.open_window(&mut table, id, now);
            }
        }
        self.activity_changed();
        self.bump();
        Ok(state)
    }
}
