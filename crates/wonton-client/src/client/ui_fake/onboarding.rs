//! 假引导：引导已经走完，首屏直接是主界面。

use wonton_protocol::{OnboardingStep, OnboardingViewModel};

pub(super) fn fake_onboarding(mut onboarding: OnboardingViewModel) -> OnboardingViewModel {
    onboarding.step = Some(OnboardingStep::Finished);
    onboarding.nav = None;
    onboarding.loading = false;
    onboarding.problem = None;
    onboarding
}
