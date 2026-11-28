//! 引导状态：假核心的应答和引导视图的等待。

use serde_json::Value;
use wonton_client::OnboardingViewModel;

use super::{Home, Phone, WAIT};

impl Home {
    /// 设置假核心的引导状态；没设置时引导接口回 404。
    pub fn set_onboarding(&self, onboarding: Value) {
        *self.core.onboarding.lock().unwrap() = Some(onboarding);
    }
}

impl Phone {
    /// 等引导的视图满足条件。
    pub async fn wait_onboarding_prompts<R>(
        &self,
        what: &str,
        mut check: impl FnMut(&OnboardingViewModel) -> Option<R>,
    ) -> R {
        let subscription = self.subscribe_onboarding();
        let waiting = async {
            while let Some(view) = subscription.next().await {
                if let Some(found) = check(&view) {
                    return found;
                }
            }
            panic!("订阅提前结束");
        };
        match tokio::time::timeout(WAIT, waiting).await {
            Ok(found) => found,
            Err(_) => panic!(
                "{what}：{WAIT:?} 内没有等到，当前：{:#?}",
                self.onboarding()
            ),
        }
    }
}
