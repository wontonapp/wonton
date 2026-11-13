//! 接模型这一步只选择沿用网关配置或官方账号登录；网关网页自己保存模型设置。

use wonton_l10n::l10n;
use wonton_protocol::{CoreError, OnboardingStep};

use super::draft::ModelDraft;
use super::{OnboardingLink, invalid, lock};
use crate::l10n::{CATALOG, language};

impl OnboardingLink {
    pub(super) fn use_current_model(&self) -> Result<(), CoreError> {
        let mut state = lock(&self.state);
        if state.step != Some(OnboardingStep::Model) {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-model-not-model-step"
            )));
        }
        let configured = state
            .providers
            .as_ref()
            .is_some_and(|providers| !providers.configured.is_empty());
        if !configured {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-model-none-configured"
            )));
        }
        state.draft.model = Some(ModelDraft::Current);
        state.model_problem = None;
        drop(state);
        self.publish();
        Ok(())
    }

    pub(super) fn choose_login(&self, id: &str) -> Result<(), CoreError> {
        let mut state = lock(&self.state);
        if state.step != Some(OnboardingStep::Model) {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-model-not-model-step"
            )));
        }
        let onboarding = state.onboarding.as_ref().ok_or_else(|| {
            invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-model-state-missing"
            ))
        })?;
        let agent = state.draft.agent(onboarding);
        let login = agent
            .and_then(|agent| onboarding.agents.iter().find(|row| row.id == agent))
            .and_then(|agent| agent.logins.iter().find(|login| login.id == id))
            .ok_or_else(|| {
                invalid(&l10n!(
                    CATALOG,
                    language(),
                    "onboarding-model-login-missing"
                ))
            })?;
        let choice = ModelDraft::Login {
            id: login.id.clone(),
            title: login.title.clone(),
        };
        state.draft.model = Some(choice);
        state.model_problem = None;
        drop(state);
        self.publish();
        Ok(())
    }
}
