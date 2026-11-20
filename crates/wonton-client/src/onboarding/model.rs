//! 接模型这一步只选择沿用网关配置；网关网页自己保存模型设置。

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
            .model_settings
            .as_ref()
            .is_some_and(|settings| settings.has_configured_models);
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
}
