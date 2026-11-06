//! 接模型这一步：测试填的服务商（只测不存），记下选择。保存在最后一页点完成时（[`super::review`]）。

use std::sync::Arc;

use wonton_protocol::{CoreError, ModelProviderFailure, ModelProviderInput, OnboardingStep};

use super::draft::{ModelDraft, key_is_empty, trimmed};
use super::{OnboardingLink, api, invalid, lock, provider_problem};

impl OnboardingLink {
    pub(super) fn test_provider(
        self: &Arc<Self>,
        input: ModelProviderInput,
        then_continue: bool,
    ) -> Result<(), CoreError> {
        let input = trimmed(input);
        if key_is_empty(&input) {
            return Err(invalid("key 不能为空"));
        }
        let title = {
            let mut state = lock(&self.state);
            if state.step != Some(OnboardingStep::Model) {
                return Err(invalid("还没到接模型这一步"));
            }
            if state.testing {
                return Err(invalid("已经在测试"));
            }
            let providers = state
                .providers
                .as_ref()
                .ok_or_else(|| invalid("还没取到服务商列表"))?;
            let title = match &input {
                ModelProviderInput::Preset { id, .. } => providers
                    .providers
                    .iter()
                    .flatten()
                    .find(|provider| provider.id == *id)
                    .map(|provider| provider.title.clone())
                    .ok_or_else(|| invalid("这个服务商不在列表里"))?,
                ModelProviderInput::Custom {
                    base_url, model, ..
                } => {
                    if !providers.custom {
                        return Err(invalid("模型网关不支持自定义服务商"));
                    }
                    if model.is_empty() {
                        return Err(invalid("模型名不能为空"));
                    }
                    url::Url::parse(base_url)
                        .ok()
                        .and_then(|url| url.host_str().map(str::to_owned))
                        .ok_or_else(|| invalid("Base URL 要是 https://… 这样的地址"))?
                }
            };
            state.testing = true;
            state.model_problem = None;
            title
        };
        self.publish();
        let link = self.clone();
        let generation = self.generation();
        self.runtime.spawn(async move {
            let result = match link.plugins.api() {
                Ok(api) => api::test_provider(&api, input.clone()).await,
                Err(problem) => Err(provider_problem(problem)),
            };
            let mut state = lock(&link.state);
            if link.generation() != generation || !state.testing {
                return;
            }
            state.testing = false;
            let reachable = match result {
                Ok(()) => Some(true),
                Err(problem) if problem.reason == ModelProviderFailure::ProviderUnreachable => {
                    Some(false)
                }
                Err(problem) => {
                    state.model_problem = Some(problem);
                    None
                }
            };
            if let Some(reachable) = reachable {
                state.draft.model = Some(ModelDraft::Tested {
                    input,
                    title,
                    reachable,
                });
                if then_continue && state.step == Some(OnboardingStep::Model) {
                    state.step = Some(OnboardingStep::Apps);
                }
            }
            drop(state);
            link.publish();
        });
        Ok(())
    }

    pub(super) fn use_current_model(&self) -> Result<(), CoreError> {
        let mut state = lock(&self.state);
        if state.step != Some(OnboardingStep::Model) || state.testing {
            return Err(invalid("不在接模型这一步，或者正在测试"));
        }
        let configured = state
            .providers
            .as_ref()
            .is_some_and(|providers| !providers.configured.is_empty());
        if !configured {
            return Err(invalid("模型网关里还没有接好的模型"));
        }
        state.draft.model = Some(ModelDraft::Current);
        state.model_problem = None;
        drop(state);
        self.publish();
        Ok(())
    }

    pub(super) fn choose_login(&self, id: &str) -> Result<(), CoreError> {
        let mut state = lock(&self.state);
        if state.step != Some(OnboardingStep::Model) || state.testing {
            return Err(invalid("不在接模型这一步，或者正在测试"));
        }
        let onboarding = state
            .onboarding
            .as_ref()
            .ok_or_else(|| invalid("还没有取到引导状态"))?;
        let agent = state.draft.agent(onboarding);
        let login = agent
            .and_then(|agent| onboarding.agents.iter().find(|row| row.id == agent))
            .and_then(|agent| agent.logins.iter().find(|login| login.id == id))
            .ok_or_else(|| invalid("选中的 Agent 没有这种登录"))?;
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
