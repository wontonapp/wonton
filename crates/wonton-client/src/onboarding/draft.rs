//! 资料、Agent 和应用的选择只在内存里，点完成才保存。模型网页不属于草稿；这里只记沿用配置或官方登录。
//! 没改过的步骤取家里服务器上的设置。

use std::collections::HashSet;

use wonton_l10n::l10n;
use wonton_protocol::{
    MAX_DISPLAY_NAME_CHARS, Me, ModelProviders, Onboarding, OnboardingAgent, OnboardingModelChoice,
    OnboardingProfileStep, OwnerProfileUpdate, ProfileProblem, check_display_name,
    check_email_change, normalized_email,
};

use super::view::installable;
use crate::l10n::{CATALOG, language};

#[derive(Default)]
pub(super) struct Draft {
    /// 改过的名字和邮箱；为空时用家里服务器上的。
    pub profile: Option<(String, String)>,
    pub profile_skipped: bool,
    pub name_problem: Option<String>,
    pub email_problem: Option<String>,
    pub agent: AgentChoice,
    /// 为空时：网关里已经接了模型就是“用现在的”，否则不动模型。
    pub model: Option<ModelDraft>,
    /// 为空时：推荐的插件都已经装着的需求算选中。
    pub needs: Option<HashSet<String>>,
}

#[derive(Default)]
pub(super) enum AgentChoice {
    /// 默认 Agent，没有时取第一个能装的。
    #[default]
    Current,
    Chosen(String),
    Skipped,
}

pub(super) enum ModelDraft {
    Current,
    Login { id: String, title: String },
    Skipped,
}

impl Draft {
    pub fn profile_values(&self, owner: Option<&Me>) -> (String, String) {
        self.profile.clone().unwrap_or_else(|| {
            owner.map_or_else(Default::default, |owner| {
                (
                    owner.display_name.clone().unwrap_or_default(),
                    owner.email.clone().unwrap_or_default(),
                )
            })
        })
    }

    pub fn profile_step(&self, owner: Option<&Me>) -> OnboardingProfileStep {
        let (display_name, email) = self.profile_values(owner);
        OnboardingProfileStep {
            display_name,
            email,
            display_name_problem: self.name_problem.clone(),
            email_problem: self.email_problem.clone(),
        }
    }

    /// 查名字和邮箱，规则和家里服务器保存时的同一份。不合规则时记下原因，返回假。
    pub fn check_profile(&mut self, owner: Option<&Me>) -> bool {
        let (name, email) = self.profile_values(owner);
        self.name_problem = check_display_name(name.trim()).err().map(problem_text);
        let previous = owner.and_then(|owner| owner.email.as_deref());
        self.email_problem = check_email_change(previous, normalized_email(Some(&email)))
            .err()
            .map(problem_text);
        self.profile_skipped = false;
        self.name_problem.is_none() && self.email_problem.is_none()
    }

    /// 要保存的资料；没跳过、和家里服务器上的不一样时才有。
    pub fn profile_update(&self, owner: Option<&Me>) -> Option<OwnerProfileUpdate> {
        if self.profile_skipped {
            return None;
        }
        let (name, email) = self.profile_values(owner);
        let name = name.trim().to_owned();
        let email = normalized_email(Some(&email)).map(str::to_owned);
        let unchanged = owner.is_some_and(|owner| {
            owner.display_name.as_deref() == Some(name.as_str()) && owner.email == email
        });
        (!unchanged).then_some(OwnerProfileUpdate {
            display_name: name,
            email,
        })
    }

    pub fn agent(&self, onboarding: &Onboarding) -> Option<String> {
        match &self.agent {
            AgentChoice::Chosen(id) => Some(id.clone()),
            AgentChoice::Skipped => None,
            AgentChoice::Current => onboarding.primary_agent.clone().or_else(|| {
                onboarding
                    .agents
                    .iter()
                    .find(|agent| installable(agent))
                    .map(|agent| agent.id.clone())
            }),
        }
    }

    /// 完成时会被设成默认 Agent 的那个；为空时完成不发 `primary_agent`，服务器上的现值不动。
    /// 重看引导里没点过 Agent（或者又点回原来那个）都不算改：首次引导的“用推荐的”才替主人定一个。
    pub fn primary_change<'a>(
        &self,
        onboarding: &'a Onboarding,
        revisit: bool,
    ) -> Option<&'a OnboardingAgent> {
        let agent = match &self.agent {
            AgentChoice::Chosen(id) => onboarding.agents.iter().find(|agent| agent.id == *id)?,
            AgentChoice::Current if !revisit => {
                let id = self.agent(onboarding)?;
                onboarding.agents.iter().find(|agent| agent.id == id)?
            }
            _ => return None,
        };
        let already_default = agent.status.is_some()
            && onboarding.primary_agent.as_deref() == Some(agent.id.as_str());
        (!already_default).then_some(agent)
    }

    pub fn model<'a>(&'a self, providers: Option<&ModelProviders>) -> Option<&'a ModelDraft> {
        let configured = providers.is_some_and(|providers| !providers.configured.is_empty());
        match &self.model {
            Some(ModelDraft::Current) | None if configured => Some(&ModelDraft::Current),
            Some(ModelDraft::Current | ModelDraft::Skipped) | None => None,
            Some(model @ ModelDraft::Login { .. }) => Some(model),
        }
    }

    pub fn needs(&self, onboarding: &Onboarding) -> HashSet<String> {
        self.needs.clone().unwrap_or_else(|| {
            onboarding
                .needs
                .iter()
                .filter(|need| {
                    !need.plugins.is_empty()
                        && need
                            .plugins
                            .iter()
                            .all(|plugin| plugin.installed_version.is_some())
                })
                .map(|need| need.id.clone())
                .collect()
        })
    }
}

impl ModelDraft {
    pub fn choice(&self) -> Option<OnboardingModelChoice> {
        Some(match self {
            Self::Current => OnboardingModelChoice::Current,
            Self::Login { id, title } => OnboardingModelChoice::Login {
                id: id.clone(),
                title: title.clone(),
            },
            Self::Skipped => return None,
        })
    }
}

fn problem_text(problem: ProfileProblem) -> String {
    let id = match problem {
        ProfileProblem::NameEmpty => "onboarding-profile-name-empty",
        ProfileProblem::NameTooLong => "onboarding-profile-name-too-long",
        ProfileProblem::NameControlChars => "onboarding-profile-name-control",
        ProfileProblem::EmailInvalid => "onboarding-profile-email-invalid",
        ProfileProblem::EmailCleared => "onboarding-profile-email-cleared",
    };
    l10n!(CATALOG, language(), id, max = MAX_DISPLAY_NAME_CHARS)
}
