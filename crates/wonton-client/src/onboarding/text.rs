//! 引导界面用的文字，按请求的语言挑好；壳只拿成品字符串。

use wonton_protocol::{AgentLogin, OnboardingStep, OnboardingTip};

#[derive(Clone, Copy)]
pub(crate) enum Language {
    Chinese,
    English,
}

pub(crate) fn language(languages: &[String]) -> Language {
    if languages
        .first()
        .is_some_and(|language| language.to_ascii_lowercase().starts_with("zh"))
    {
        Language::Chinese
    } else {
        Language::English
    }
}

pub(crate) fn title(step: OnboardingStep, language: Language) -> String {
    let (zh, en) = match step {
        OnboardingStep::Profile => ("你的资料", "Your profile"),
        OnboardingStep::Agent => ("选 Agent", "Choose an agent"),
        OnboardingStep::Model => ("接模型", "Connect a model"),
        OnboardingStep::Apps => ("选应用", "Choose apps"),
        OnboardingStep::Review => ("确认", "Review"),
        OnboardingStep::Finished => ("完成", "Finished"),
        OnboardingStep::Other => ("引导", "Onboarding"),
    };
    choose(language, zh, en)
}

pub(crate) fn connection(logins: &[AgentLogin], language: Language) -> String {
    if logins.is_empty() {
        return choose(language, "key", "key");
    }
    let titles: Vec<&str> = logins.iter().map(|login| login.title.as_str()).collect();
    match language {
        Language::Chinese => format!("{}或 key", titles.join("、")),
        Language::English => format!("{} or key", titles.join(", ")),
    }
}

pub(crate) fn tip(tip: OnboardingTip, language: Language) -> String {
    let (zh, en) = match tip {
        OnboardingTip::ConfirmationCard => (
            "需要确认时，在聊天里的卡片上点确认。",
            "Confirm requests from the card in chat.",
        ),
        OnboardingTip::AppInstalled => (
            "应用装好后，可以把它放到底栏。",
            "You can pin an installed app to the bar.",
        ),
    };
    choose(language, zh, en)
}

fn choose(language: Language, zh: &str, en: &str) -> String {
    match language {
        Language::Chinese => zh.to_owned(),
        Language::English => en.to_owned(),
    }
}
