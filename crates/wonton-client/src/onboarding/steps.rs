//! 在步骤之间走：继续、跳过、返回、重看都只改这台设备上记下的选择（ADR-0038），点完成之前不发保存请求。

use std::sync::Arc;

use wonton_l10n::l10n;
use wonton_protocol::{CoreError, OnboardingCommit, OnboardingStep};

use super::draft::{AgentChoice, ModelDraft};
use super::{OnboardingLink, invalid, lock, view};
use crate::l10n::{CATALOG, language};

impl OnboardingLink {
    /// `step` 之后第一个还能显示的步；没有了为空。
    fn successor(state: &super::LocalState, step: OnboardingStep) -> Option<OnboardingStep> {
        let index = view::STEPS
            .iter()
            .position(|candidate| *candidate == step)?;
        view::STEPS[index + 1..]
            .iter()
            .copied()
            .find(|candidate| !state.unavailable.contains(candidate))
    }

    /// 从这个候选往前走，跳过服务器太旧显不出来的步。
    fn advance(state: &super::LocalState, mut next: OnboardingStep) -> OnboardingStep {
        while state.unavailable.contains(&next) {
            match Self::successor(state, next) {
                Some(after) => next = after,
                None => break,
            }
        }
        next
    }

    /// 正在走引导的一步；走完了、还没取到状态或者已经点了完成时为错。
    fn flow_step(state: &super::LocalState) -> Result<OnboardingStep, CoreError> {
        match state.step {
            Some(OnboardingStep::Finished | OnboardingStep::Other) | None => Err(invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-steps-not-in-flow"
            ))),
            Some(_) if state.commit != OnboardingCommit::NotStarted => Err(invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-steps-already-committed"
            ))),
            Some(step) => Ok(step),
        }
    }

    pub(super) fn edit_profile(
        &self,
        display_name: String,
        email: String,
    ) -> Result<(), CoreError> {
        let mut state = lock(&self.state);
        if Self::flow_step(&state)? != OnboardingStep::Profile {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-steps-not-profile-step"
            )));
        }
        state.draft.profile = Some((display_name, email));
        // 已经提示过不合规则的，改的时候跟着查，改对了提示就消失。
        if state.draft.name_problem.is_some() || state.draft.email_problem.is_some() {
            let owner = state.owner.clone();
            state.draft.check_profile(owner.as_ref());
        }
        drop(state);
        self.publish();
        Ok(())
    }

    pub(super) fn choose_agent(&self, id: &str) -> Result<(), CoreError> {
        let mut state = lock(&self.state);
        if Self::flow_step(&state)? != OnboardingStep::Agent {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-steps-not-agent-step"
            )));
        }
        let onboarding = state.onboarding.as_ref().ok_or_else(|| {
            invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-steps-state-missing"
            ))
        })?;
        let agent = onboarding
            .agents
            .iter()
            .find(|agent| agent.id == id)
            .ok_or_else(|| {
                invalid(&l10n!(
                    CATALOG,
                    language(),
                    "onboarding-steps-agent-not-listed"
                ))
            })?;
        if !view::installable(agent) {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-steps-agent-not-installable"
            )));
        }
        state.draft.agent = AgentChoice::Chosen(id.to_owned());
        drop(state);
        self.publish();
        Ok(())
    }

    pub(super) fn toggle_need(&self, id: &str) -> Result<(), CoreError> {
        let mut state = lock(&self.state);
        if Self::flow_step(&state)? != OnboardingStep::Apps {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-steps-not-apps-step"
            )));
        }
        let onboarding = state.onboarding.as_ref().ok_or_else(|| {
            invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-steps-state-missing"
            ))
        })?;
        if !onboarding.needs.iter().any(|need| need.id == id) {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-steps-need-not-listed"
            )));
        }
        let mut needs = state.draft.needs(onboarding);
        if !needs.insert(id.to_owned()) {
            needs.remove(id);
        }
        state.draft.needs = Some(needs);
        drop(state);
        self.publish();
        Ok(())
    }

    pub(super) fn continue_step(self: &Arc<Self>) -> Result<(), CoreError> {
        let mut state = lock(&self.state);
        if state.step == Some(OnboardingStep::Review) {
            return match state.commit {
                OnboardingCommit::NotStarted => {
                    drop(state);
                    self.commit()
                }
                OnboardingCommit::Running => Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "onboarding-steps-committing"
                ))),
                OnboardingCommit::Done => {
                    state.step = Some(OnboardingStep::Finished);
                    drop(state);
                    self.publish();
                    Ok(())
                }
            };
        }
        let next = match Self::flow_step(&state)? {
            OnboardingStep::Profile => {
                let owner = state.owner.clone();
                if !state.draft.check_profile(owner.as_ref()) {
                    // 不合规则的原因显示在输入框下面，停在这一步。
                    drop(state);
                    self.publish();
                    return Ok(());
                }
                OnboardingStep::Agent
            }
            OnboardingStep::Agent => {
                let onboarding = state.onboarding.as_ref().ok_or_else(|| {
                    invalid(&l10n!(
                        CATALOG,
                        language(),
                        "onboarding-steps-state-missing"
                    ))
                })?;
                let installable = state.draft.agent(onboarding).is_some_and(|id| {
                    onboarding
                        .agents
                        .iter()
                        .any(|agent| agent.id == id && view::installable(agent))
                });
                if !installable {
                    return Err(invalid(&l10n!(
                        CATALOG,
                        language(),
                        "onboarding-steps-no-installable-agent"
                    )));
                }
                OnboardingStep::Model
            }
            OnboardingStep::Model => {
                if state.draft.model(state.providers.as_ref()).is_none() {
                    return Err(invalid(&l10n!(
                        CATALOG,
                        language(),
                        "onboarding-steps-model-not-chosen"
                    )));
                }
                OnboardingStep::Apps
            }
            OnboardingStep::Apps => {
                drop(state);
                self.enter_review();
                return Ok(());
            }
            OnboardingStep::Review | OnboardingStep::Finished | OnboardingStep::Other => {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "onboarding-steps-not-in-flow"
                )));
            }
        };
        state.step = Some(Self::advance(&state, next));
        drop(state);
        self.publish();
        Ok(())
    }

    pub(super) fn skip_step(self: &Arc<Self>) -> Result<(), CoreError> {
        let mut state = lock(&self.state);
        let next = match Self::flow_step(&state)? {
            OnboardingStep::Profile => {
                let draft = &mut state.draft;
                draft.profile = None;
                draft.profile_skipped = true;
                draft.name_problem = None;
                draft.email_problem = None;
                OnboardingStep::Agent
            }
            OnboardingStep::Agent => {
                state.draft.agent = AgentChoice::Skipped;
                OnboardingStep::Model
            }
            OnboardingStep::Model => {
                state.model_problem = None;
                state.draft.model = Some(ModelDraft::Skipped);
                OnboardingStep::Apps
            }
            OnboardingStep::Apps => {
                state.draft.needs = Some(Default::default());
                drop(state);
                self.enter_review();
                return Ok(());
            }
            OnboardingStep::Review | OnboardingStep::Finished | OnboardingStep::Other => {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "onboarding-steps-cannot-skip"
                )));
            }
        };
        state.step = Some(Self::advance(&state, next));
        drop(state);
        self.publish();
        Ok(())
    }

    pub(super) fn back_step(&self) -> Result<(), CoreError> {
        let mut state = lock(&self.state);
        let step = Self::flow_step(&state)?;
        let mut target = match step {
            OnboardingStep::Agent => OnboardingStep::Profile,
            OnboardingStep::Model => OnboardingStep::Agent,
            OnboardingStep::Apps => OnboardingStep::Model,
            OnboardingStep::Review => {
                // 旧预览的结果按 review_generation 丢掉。
                state.review.clear();
                state.review_generation += 1;
                OnboardingStep::Apps
            }
            OnboardingStep::Profile | OnboardingStep::Finished | OnboardingStep::Other => {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "onboarding-steps-cannot-go-back"
                )));
            }
        };
        if state.unavailable.contains(&target) {
            let index = view::STEPS
                .iter()
                .position(|candidate| *candidate == target)
                .unwrap_or_default();
            match view::STEPS[..index]
                .iter()
                .rev()
                .find(|candidate| !state.unavailable.contains(candidate))
            {
                Some(before) => target = *before,
                None => {
                    return Err(invalid(&l10n!(
                        CATALOG,
                        language(),
                        "onboarding-steps-cannot-go-back"
                    )));
                }
            }
        }
        state.step = Some(target);
        drop(state);
        self.publish();
        Ok(())
    }

    /// 重看引导：从第一步开始，每一步显示家里服务器上现在的设置。
    pub(super) fn restart(self: &Arc<Self>) -> Result<(), CoreError> {
        {
            let mut state = lock(&self.state);
            if state.onboarding.is_none() {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "onboarding-steps-state-missing"
                )));
            }
            if state.commit == OnboardingCommit::Running {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "onboarding-steps-committing"
                )));
            }
            state.revisit = state
                .onboarding
                .as_ref()
                .is_some_and(|onboarding| onboarding.step == OnboardingStep::Finished);
            state.draft = Default::default();
            state.review.clear();
            state.review_generation += 1;
            state.commit = OnboardingCommit::NotStarted;
            state.step = view::first_visible(&state.unavailable).or(Some(OnboardingStep::Profile));
        }
        self.refresh_after_local_reset();
        Ok(())
    }

    /// 关掉重看的引导：这次记下的选择全部丢弃，什么都不发；`step` 清空让壳回到设置。
    pub(crate) fn close(&self) {
        {
            let mut state = lock(&self.state);
            if state.commit == OnboardingCommit::Running {
                return;
            }
            state.draft = Default::default();
            state.review.clear();
            state.review_generation += 1;
            state.commit = OnboardingCommit::NotStarted;
            state.model_problem = None;
            state.step = None;
        }
        self.publish();
    }
}
