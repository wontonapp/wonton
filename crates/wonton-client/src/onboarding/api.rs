//! 引导状态、资料和助手名字的接口调用。安装预览走 `plugins`，模型页的操作不经这里。

use wonton_protocol::{
    BotProfileUpdate, BotProfileUpdated, Me, Onboarding, OnboardingUpdate, OwnerProfileUpdate,
    PluginProblem,
};

use crate::plugins::api::{CoreApi, READ_TIMEOUT, SOURCE_TIMEOUT};

pub(crate) async fn get(api: &CoreApi) -> Result<Onboarding, PluginProblem> {
    api.get(&["onboarding"], &[], SOURCE_TIMEOUT).await
}

pub(crate) async fn update(
    api: &CoreApi,
    update: &OnboardingUpdate,
) -> Result<Onboarding, PluginProblem> {
    api.put(&["onboarding"], update, SOURCE_TIMEOUT).await
}

pub(crate) async fn me(api: &CoreApi) -> Result<Me, PluginProblem> {
    api.get(&["me"], &[], READ_TIMEOUT).await
}

pub(crate) async fn save_me(
    api: &CoreApi,
    update: &OwnerProfileUpdate,
) -> Result<Me, PluginProblem> {
    api.put(&["me"], update, READ_TIMEOUT).await
}

/// 给默认 Bot 改名（[`wonton_protocol::bot_profile_path`]）。
pub(crate) async fn rename_bot(
    api: &CoreApi,
    plugin_id: &str,
    name: String,
) -> Result<BotProfileUpdated, PluginProblem> {
    let update = BotProfileUpdate {
        name: Some(name),
        ..BotProfileUpdate::default()
    };
    api.put(&["bots", plugin_id, "profile"], &update, READ_TIMEOUT)
        .await
}
