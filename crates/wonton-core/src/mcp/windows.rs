//! 放行时段的总线这一侧：回答卡片时打开，到期或主人停止时结束，每一步写审计（ADR-0024）。规则在
//! [`super::confirm`]。

use std::sync::Arc;
use std::time::Duration;

use wonton_protocol::{Answer, ConfirmationState, SecretValues, WindowEnd};

use super::Bus;
use super::audit::WindowRecord;
use super::confirm::{AnswerError, EndedWindow};

/// 上次运行的卡片还改不了时（Matrix 没好），隔这么久再试。
const RECOVER_RETRY: Duration = Duration::from_secs(60);

impl Bus {
    /// 主人回答卡片。`by` 是谁回答的。
    pub fn answer_card(
        self: &Arc<Self>,
        id: &str,
        answer: Answer,
        choice_id: Option<&str>,
        by: &str,
    ) -> Result<ConfirmationState, AnswerError> {
        self.answer_card_with_secrets(
            id,
            answer,
            choice_id,
            SecretValues::new(),
            Default::default(),
            by,
        )
    }

    /// 带密钥的回答卡片（ADR-0064）。
    pub fn answer_card_with_secrets(
        self: &Arc<Self>,
        id: &str,
        answer: Answer,
        choice_id: Option<&str>,
        secrets: SecretValues,
        settings: wonton_protocol::SettingValues,
        by: &str,
    ) -> Result<ConfirmationState, AnswerError> {
        let state = self.confirmations.answer_with_secrets_by(
            id,
            answer,
            choice_id,
            secrets,
            settings,
            &self.owner_languages(),
            Some(by),
        )?;
        if answer == Answer::AllowWindow {
            self.window_opened(id, by);
        }
        Ok(state)
    }

    fn window_opened(self: &Arc<Self>, id: &str, by: &str) {
        if let Some(opened) = self.confirmations.opened_window(id) {
            self.audit.record(&WindowRecord {
                window: "opened",
                card: id,
                caller: &opened.caller,
                plugin: &opened.plugin,
                by: Some(by),
                allowed: Some(opened.released),
                reason: None,
            });
        }
        // 判定自己看到期时间，这个任务只管改卡片、写审计。
        let (bus, id, length) = (
            self.clone(),
            id.to_owned(),
            self.confirmations.window_length(),
        );
        tokio::spawn(async move {
            tokio::time::sleep(length).await;
            bus.end_window(&id, WindowEnd::Expired, None).await;
        });
    }

    /// 结束时段。没有这个时段或者已经结束时返回 `false`。
    pub async fn end_window(&self, id: &str, reason: WindowEnd, by: Option<&str>) -> bool {
        let Some(ended) = self.confirmations.end_window(id, reason).await else {
            return false;
        };
        self.audit_ended(&ended, by);
        true
    }

    fn audit_ended(&self, ended: &EndedWindow, by: Option<&str>) {
        self.audit.record(&WindowRecord {
            window: "ended",
            card: &ended.card,
            caller: &ended.caller,
            plugin: &ended.plugin,
            by,
            allowed: ended.allowed,
            reason: Some(ended.reason),
        });
    }

    /// 把上次运行留下的放行卡片改成已结束。核心刚起来时 Matrix 可能没好，改不了就隔一会再试。
    pub(super) fn recover_windows(self: &Arc<Self>) {
        let bus = self.clone();
        tokio::spawn(async move {
            loop {
                let langs = bus.plugins.owner_languages();
                for ended in bus.confirmations.recover_windows(&langs).await {
                    bus.audit_ended(&ended, None);
                }
                if !bus.confirmations.has_leftover_windows() {
                    return;
                }
                tokio::time::sleep(RECOVER_RETRY).await;
            }
        });
    }
}
