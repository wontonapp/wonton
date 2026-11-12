//! 在步骤之间走：继续、跳过、返回、重看都只改这台设备上记下的选择（ADR-0038），点完成之前不发保存请求。

use std::sync::Arc;

use wonton_protocol::{CoreError, OnboardingCommit, OnboardingStep};

use super::draft::{AgentChoice, ModelDraft};
use super::{OnboardingLink, invalid, lock, view};

impl OnboardingLink {
    /// 正在走引导的一步；走完了、还没取到状态或者已经点了完成时为错。
    fn flow_step(state: &super::LocalState) -> Result<OnboardingStep, CoreError> {
        match state.step {
            Some(OnboardingStep::Finished | OnboardingStep::Other) | None => {
                Err(invalid("不在引导里"))
            }
            Some(_) if state.commit != OnboardingCommit::NotStarted => Err(invalid("已经点了完成")),
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
            return Err(invalid("不在填资料这一步"));
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
            return Err(invalid("不在选 Agent 这一步"));
        }
        let onboarding = state
            .onboarding
            .as_ref()
            .ok_or_else(|| invalid("还没有取到引导状态"))?;
        let agent = onboarding
            .agents
            .iter()
            .find(|agent| agent.id == id)
            .ok_or_else(|| invalid("这个 Agent 不在可选列表里"))?;
        if !view::installable(agent) {
            return Err(invalid("这个 Agent 不能安装"));
        }
        state.draft.agent = AgentChoice::Chosen(id.to_owned());
        drop(state);
        self.publish();
        Ok(())
    }

    pub(super) fn toggle_need(&self, id: &str) -> Result<(), CoreError> {
        let mut state = lock(&self.state);
        if Self::flow_step(&state)? != OnboardingStep::Apps {
            return Err(invalid("不在选应用这一步"));
        }
        let onboarding = state
            .onboarding
            .as_ref()
            .ok_or_else(|| invalid("还没有取到引导状态"))?;
        if !onboarding.needs.iter().any(|need| need.id == id) {
            return Err(invalid("这个需求不在推荐列表里"));
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
                OnboardingCommit::Running => Err(invalid("正在提交")),
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
                let onboarding = state
                    .onboarding
                    .as_ref()
                    .ok_or_else(|| invalid("还没有取到引导状态"))?;
                let installable = state.draft.agent(onboarding).is_some_and(|id| {
                    onboarding
                        .agents
                        .iter()
                        .any(|agent| agent.id == id && view::installable(agent))
                });
                if !installable {
                    return Err(invalid("没有选能装的 Agent"));
                }
                OnboardingStep::Model
            }
            OnboardingStep::Model => {
                if state.testing {
                    return Err(invalid("正在测试"));
                }
                if state.draft.model(state.providers.as_ref()).is_none() {
                    return Err(invalid("还没有选模型"));
                }
                OnboardingStep::Apps
            }
            OnboardingStep::Apps => {
                drop(state);
                self.enter_review();
                return Ok(());
            }
            OnboardingStep::Review | OnboardingStep::Finished | OnboardingStep::Other => {
                return Err(invalid("不在引导里"));
            }
        };
        state.step = Some(next);
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
                // 测试的结果看 testing，清掉后到了也不落下。
                state.testing = false;
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
                return Err(invalid("这一步不能跳过"));
            }
        };
        state.step = Some(next);
        drop(state);
        self.publish();
        Ok(())
    }

    pub(super) fn back_step(&self) -> Result<(), CoreError> {
        let mut state = lock(&self.state);
        let step = Self::flow_step(&state)?;
        state.step = Some(match step {
            OnboardingStep::Agent => OnboardingStep::Profile,
            OnboardingStep::Model => {
                state.testing = false;
                OnboardingStep::Agent
            }
            OnboardingStep::Apps => OnboardingStep::Model,
            OnboardingStep::Review => {
                // 旧预览的结果按 review_generation 丢掉。
                state.review.clear();
                state.review_generation += 1;
                OnboardingStep::Apps
            }
            OnboardingStep::Profile | OnboardingStep::Finished | OnboardingStep::Other => {
                return Err(invalid("这一步不能返回"));
            }
        });
        drop(state);
        self.publish();
        Ok(())
    }

    /// 重看引导：从第一步开始，每一步显示家里服务器上现在的设置。
    pub(super) fn restart(self: &Arc<Self>) -> Result<(), CoreError> {
        {
            let mut state = lock(&self.state);
            if state.onboarding.is_none() {
                return Err(invalid("还没有取到引导状态"));
            }
            if state.commit == OnboardingCommit::Running {
                return Err(invalid("正在提交"));
            }
            state.draft = Default::default();
            state.review.clear();
            state.review_generation += 1;
            state.commit = OnboardingCommit::NotStarted;
            state.testing = false;
            state.step = Some(OnboardingStep::Profile);
        }
        self.refresh();
        Ok(())
    }
}
