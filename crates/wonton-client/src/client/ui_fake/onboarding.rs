//! 假引导：引导已经走完，首屏直接是主界面。
//!
//! `WONTON_UI_FAKE_ONBOARDING=transport_problem` 时取引导状态失败成“连不上”：设置页顶上有提示和“重试”。
//! 点重试像真核心那样先进行中（提示还在、`loading` 为真），过一会儿取到、提示收掉。

use std::time::Duration;

use wonton_protocol::{
    DefaultBotVisibility, OnboardingAction, OnboardingStep, OnboardingViewModel, PluginFailure,
    PluginProblem,
};

use super::super::ClientCore;

/// 假重试从进行中到取到的时间：够界面测试看到转圈。
const FAKE_RETRY: Duration = Duration::from_millis(1500);

pub(super) fn fake_onboarding(mut onboarding: OnboardingViewModel) -> OnboardingViewModel {
    onboarding.step = Some(OnboardingStep::Finished);
    // 假数据里已经有一个可见的默认 Bot（@echo:home.example），和它的私聊就在假房间里。
    onboarding.default_bot_user_id = Some("@echo:home.example".to_owned());
    onboarding.default_bot_visibility = Some(DefaultBotVisibility::Visible);
    onboarding.nav = None;
    onboarding.loading = false;
    onboarding.problem = (std::env::var("WONTON_UI_FAKE_ONBOARDING").as_deref()
        == Ok("transport_problem"))
    .then(|| PluginProblem {
        reason: PluginFailure::TransportUnavailable,
        min_version: None,
        message: "Fake request dropped on the way".to_owned(),
        settings: None,
    });
    onboarding
}

impl ClientCore {
    pub(in crate::client) fn dispatch_ui_fake_onboarding(&self, action: OnboardingAction) {
        if action != OnboardingAction::RefreshOnboarding {
            return;
        }
        let view = self.views().onboarding.clone();
        view.send_modify(|onboarding| onboarding.loading = true);
        std::thread::spawn(move || {
            std::thread::sleep(FAKE_RETRY);
            view.send_modify(|onboarding| {
                onboarding.loading = false;
                onboarding.problem = None;
            });
        });
    }
}
