use serde::{Deserialize, Serialize};

use crate::OnboardingTip;

/// 新用户引导操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OnboardingAction {
    /// 取引导的状态、推荐清单和资料。
    RefreshOnboarding,
    /// 主人在设置里选了重看：从头走一遍。模型页里存过的不属于引导草稿。
    RestartOnboarding,
    /// 记下这一步的选择，到下一步。最后一页上是“完成”：一起执行所有选择；执行完了再发一次是离开引导。
    ContinueOnboarding,
    /// 跳过这一步，不算做完：没接模型时默认 Bot 的私聊照常说明要接模型。
    SkipOnboardingStep,
    /// 回到上一步，记下的选择还在。
    BackOnboardingStep,
    /// 选中或者取消一项需求。选中的需求推荐的插件，点完成时安装。
    ToggleOnboardingNeed { id: String },
    /// 你的资料这一步改了名字或邮箱。只记下来，点完成时才保存。
    EditOnboardingProfile { display_name: String, email: String },
    /// 你的助手这一步改了名字。只记下来，点完成时才保存。
    EditOnboardingAssistant { name: String },
    /// 最后一页上重试失败的一项（[`OnboardingReviewItem::id`](crate::OnboardingReviewItem::id)）。
    RetryOnboardingItem { id: String },
    /// 这条提示看过了，这台设备不再显示。
    DismissOnboardingTip { tip: OnboardingTip },
    /// 退出从设置重看的引导并丢弃未保存的选择；未在重看时无效果。
    CloseOnboarding,
    /// Agent 的出场点过或播完了：记下见过（[`Onboarding::agent_intro`](crate::Onboarding::agent_intro)）。幂等。
    FinishAgentIntro,
}
