//! 引导的接口调用：状态和步骤经 `onboarding`，服务商列表和 key 经 `models/providers`。这几个接口在核心里都可能
//! 去来源取官方索引和图标，超时按取来源算（[`SOURCE_TIMEOUT`]）；错误统一映射成 [`ModelProviderProblem`]。

use wonton_protocol::{
    ModelProviderFailure, ModelProviderKey, ModelProviderProblem, ModelProviders, Onboarding,
    OnboardingUpdate, PluginProblem,
};

use crate::plugins::api::{CoreApi, CoreApiError, SOURCE_TIMEOUT};

pub(crate) async fn get(api: &CoreApi) -> Result<Onboarding, PluginProblem> {
    api.get(&["onboarding"], &[], SOURCE_TIMEOUT).await
}

pub(crate) async fn update(
    api: &CoreApi,
    update: &OnboardingUpdate,
) -> Result<Onboarding, PluginProblem> {
    api.put(&["onboarding"], update, SOURCE_TIMEOUT).await
}

pub(crate) async fn providers(api: &CoreApi) -> Result<ModelProviders, ModelProviderProblem> {
    api.get_detailed(&["models", "providers"], &[], SOURCE_TIMEOUT)
        .await
        .map_err(map_error)
}

pub(crate) async fn save_provider(
    api: &CoreApi,
    id: &str,
    key: ModelProviderKey,
) -> Result<(), ModelProviderProblem> {
    api.put_unit_detailed(&["models", "providers", id], &key, SOURCE_TIMEOUT)
        .await
        .map_err(map_error)
}

/// 保存 key 和取服务商列表共用一张映射：核心给的两个接口用同一批错误类别。
fn map_error(error: CoreApiError) -> ModelProviderProblem {
    let reason = match &error {
        CoreApiError::Transport(_) => ModelProviderFailure::TransportUnavailable,
        CoreApiError::Protocol(_) => ModelProviderFailure::Protocol,
        CoreApiError::Http { status, kind, .. } => match kind.as_deref() {
            Some("invalid_input") => ModelProviderFailure::KeyRejected,
            Some("upstream_unavailable") => ModelProviderFailure::ProviderUnreachable,
            Some("models_unavailable") => ModelProviderFailure::GatewayNotReady,
            Some("unauthenticated" | "cross_site") => ModelProviderFailure::Unauthorized,
            Some("protocol_mismatch" | "unsupported") => ModelProviderFailure::Protocol,
            Some(_) if *status >= 500 => ModelProviderFailure::Upstream,
            Some(_) => ModelProviderFailure::Protocol,
            None => match status {
                401 | 403 => ModelProviderFailure::Unauthorized,
                502 => ModelProviderFailure::TransportUnavailable,
                500..=599 => ModelProviderFailure::Upstream,
                _ => ModelProviderFailure::Protocol,
            },
        },
    };
    ModelProviderProblem {
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
