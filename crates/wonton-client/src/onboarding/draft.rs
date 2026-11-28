//! 资料、助手的名字和应用的选择只在内存里，点完成才保存。模型页里的操作当场存进 Bot，不属于草稿。
//! 没改过的步骤取家里服务器上的设置。

use std::collections::HashSet;

use wonton_l10n::l10n;
use wonton_protocol::{
    BotNameProblem, MAX_DISPLAY_NAME_CHARS, Me, Onboarding, OnboardingAssistantStep,
    OnboardingProfileStep, OwnerProfileUpdate, ProfileProblem, check_bot_name, check_display_name,
    check_email_change, normalized_email,
};

use crate::l10n::{CATALOG, language};

#[derive(Default)]
pub(super) struct Draft {
    /// 改过的名字和邮箱；为空时用家里服务器上的。
    pub profile: Option<(String, String)>,
    pub profile_skipped: bool,
    pub name_problem: Option<String>,
    pub email_problem: Option<String>,
    /// 改过的助手名字；为空时用 Bot 现在的。
    pub assistant: Option<String>,
    pub assistant_problem: Option<BotNameProblem>,
    /// 为空时：推荐的插件都已经装着的需求算选中。
    pub needs: Option<HashSet<String>>,
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

    pub fn assistant_step(&self, current: Option<&str>) -> OnboardingAssistantStep {
        OnboardingAssistantStep {
            name: self
                .assistant
                .clone()
                .unwrap_or_else(|| current.unwrap_or_default().to_owned()),
            name_problem: self.assistant_problem,
        }
    }

    /// 查名字，规则和家里服务器保存时的同一份。没改过的不查：不改就不发请求。
    pub fn check_assistant(&mut self) -> bool {
        self.assistant_problem = self
            .assistant
            .as_deref()
            .and_then(|name| check_bot_name(name.trim()).err());
        self.assistant_problem.is_none()
    }

    /// 要保存的名字；改过、和 Bot 现在的不一样时才有。
    pub fn assistant_update(&self, current: Option<&str>) -> Option<String> {
        let name = self.assistant.as_deref()?.trim();
        (Some(name) != current).then(|| name.to_owned())
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
