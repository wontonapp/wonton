//! 引导状态、资料、模型设置网页和安装预览的接口调用。网页里的模型修改不经这里保存。

use wonton_protocol::{
    InstallPreview, InstallSource, Me, ModelSettings, ModelSettingsFailure, ModelSettingsProblem,
    Onboarding, OnboardingUpdate, OwnerProfileUpdate, PluginProblem,
};

use crate::plugins::api::{CoreApi, CoreApiError, READ_TIMEOUT, SOURCE_TIMEOUT};

pub(crate) async fn get(api: &CoreApi) -> Result<Onboarding, PluginProblem> {
    api.get(&["onboarding"], &[], SOURCE_TIMEOUT).await
}

pub(crate) async fn update(
    api: &CoreApi,
    update: &OnboardingUpdate,
) -> Result<Onboarding, PluginProblem> {
    api.put(&["onboarding"], update, SOURCE_TIMEOUT).await
}

pub(crate) async fn settings(api: &CoreApi) -> Result<ModelSettings, ModelSettingsProblem> {
    api.get_detailed(&["models", "settings"], &[], SOURCE_TIMEOUT)
        .await
        .map_err(map_error)
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

pub(crate) async fn preview(api: &CoreApi, source: &str) -> Result<InstallPreview, PluginProblem> {
    let source = InstallSource {
        source: source.to_owned(),
    };
    api.post(&["plugins", "preview"], &source, SOURCE_TIMEOUT)
        .await
}

fn map_error(error: CoreApiError) -> ModelSettingsProblem {
    let reason = match &error {
        CoreApiError::Transport(_) => ModelSettingsFailure::TransportUnavailable,
        CoreApiError::Protocol(_) => ModelSettingsFailure::Protocol,
        CoreApiError::Http { status, kind, .. } => match kind.as_deref() {
            Some("models_unavailable") => ModelSettingsFailure::GatewayNotReady,
            Some("unauthenticated" | "cross_site") => ModelSettingsFailure::Unauthorized,
            Some("protocol_mismatch" | "unsupported") => ModelSettingsFailure::Protocol,
            Some(_) if *status >= 500 => ModelSettingsFailure::Upstream,
            Some(_) => ModelSettingsFailure::Protocol,
            None => match status {
                401 | 403 => ModelSettingsFailure::Unauthorized,
                502 => ModelSettingsFailure::TransportUnavailable,
                500..=599 => ModelSettingsFailure::Upstream,
                _ => ModelSettingsFailure::Protocol,
            },
        },
    };
    ModelSettingsProblem {
        reason,
        message: error_message(&error),
    }
}

fn error_message(error: &CoreApiError) -> String {
    match error {
        CoreApiError::Transport(problem) => problem.message.clone(),
        CoreApiError::Protocol(message) => message.clone(),
        CoreApiError::Http { message, .. } => message.clone(),
    }
}
