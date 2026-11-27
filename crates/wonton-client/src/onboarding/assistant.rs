//! 你的助手和接模型这两步都对着默认 Bot：默认 Bot 插件上那个随插件装好的 Bot。

use wonton_protocol::{
    AgentModelsViewModel, BotRef, ChatViewModel, DEFAULT_BOT, ModelChoice, ModelTestResult,
    ModelTestState, OnboardingReviewDetail, OnboardingReviewKind,
};

use super::LocalState;
use super::review::ReviewEntry;

pub(super) const ASSISTANT: &str = "assistant";
pub(super) const MODEL: &str = "model";

pub(super) fn default_bot(state: &LocalState) -> Option<BotRef> {
    let plugin = state.onboarding.as_ref()?.default_bot_plugin.as_ref()?;
    Some(BotRef {
        plugin_id: plugin.id.clone(),
        bot: DEFAULT_BOT.to_owned(),
    })
}

/// 默认 Bot 现在的名字：聊天里同步到的 Bot 资料；还没同步到时为空。
pub(super) fn current_name(state: &LocalState, chat: &ChatViewModel) -> Option<String> {
    let id = state
        .onboarding
        .as_ref()?
        .default_bot_plugin
        .as_ref()?
        .bot_user_id
        .as_deref()?;
    chat.bots
        .iter()
        .find(|bot| bot.user_id == id)
        .map(|bot| bot.name.clone())
}

/// 模型页上默认 Bot 选好的主模型；模型页看的不是默认 Bot 时为空。
pub(super) fn chosen_model<'a>(
    state: &LocalState,
    models: &'a AgentModelsViewModel,
) -> Option<&'a ModelChoice> {
    let bot = default_bot(state)?;
    models
        .current
        .as_ref()
        .filter(|_| models.bot.as_ref() == Some(&bot))
}

pub(super) fn assistant_entry(state: &LocalState, chat: &ChatViewModel) -> ReviewEntry {
    let current = current_name(state, chat);
    match state.draft.assistant_update(current.as_deref()) {
        Some(name) => ReviewEntry::new(ASSISTANT, OnboardingReviewKind::Assistant, vec![name]),
        None => ReviewEntry::unchanged(
            ASSISTANT,
            OnboardingReviewKind::Assistant,
            current.into_iter().collect(),
        ),
    }
}

/// 模型页里当场存过，这一行只列出来：服务商的名字、模型和最近一次测试。
pub(super) fn model_entry(state: &LocalState, models: &AgentModelsViewModel) -> ReviewEntry {
    let Some(choice) = chosen_model(state, models) else {
        return ReviewEntry::unchanged(MODEL, OnboardingReviewKind::Model, Vec::new());
    };
    let provider = models
        .recommended
        .iter()
        .chain(&models.more)
        .chain(models.provider.as_ref().map(|page| &page.row))
        .find(|row| row.id == choice.provider)
        .map_or_else(|| choice.provider.clone(), |row| row.name.clone());
    let test = match &models.test {
        Some(ModelTestState::Done { result }) => Some(result.result),
        _ => None::<ModelTestResult>,
    };
    let mut entry = ReviewEntry::new(
        MODEL,
        OnboardingReviewKind::Model,
        vec![provider, choice.model.clone()],
    );
    entry.detail = Some(OnboardingReviewDetail::Model { test });
    entry
}
