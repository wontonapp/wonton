//! 假引导：引导已经走完，首屏直接是主界面。

use wonton_protocol::{DefaultBotVisibility, OnboardingStep, OnboardingViewModel};

pub(super) fn fake_onboarding(mut onboarding: OnboardingViewModel) -> OnboardingViewModel {
    onboarding.step = Some(OnboardingStep::Finished);
    // 假数据里已经有一个可见的默认 Bot（@echo:home.example），和它的私聊就在假房间里。
    onboarding.default_bot_user_id = Some("@echo:home.example".to_owned());
    onboarding.default_bot_visibility = Some(DefaultBotVisibility::Visible);
    onboarding.nav = None;
    onboarding.loading = false;
    onboarding.problem = None;
    onboarding
}
