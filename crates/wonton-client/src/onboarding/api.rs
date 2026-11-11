//! 引导的接口调用：状态经 `onboarding`，资料经 `me`（ME_PATH，每个人的资料），服务商经 `models/`，要装的插件经安装预览。这几个接口在
//! 核心里都可能去来源取官方索引和图标，超时按取来源算（[`SOURCE_TIMEOUT`]）；模型的错误统一映射成
//! [`ModelProviderProblem`]。

use wonton_protocol::{
    CustomModelProvider, InstallPreview, InstallSource, Me, ModelProviderFailure,
    ModelProviderInput, ModelProviderKey, ModelProviderProblem, ModelProviders, Onboarding,
    OnboardingUpdate, OwnerProfileUpdate, PluginProblem,
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

pub(crate) async fn providers(api: &CoreApi) -> Result<ModelProviders, ModelProviderProblem> {
    api.get_detailed(&["models", "providers"], &[], SOURCE_TIMEOUT)
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

/// 只测试，什么都不保存。
pub(crate) async fn test_provider(
    api: &CoreApi,
    input: ModelProviderInput,
) -> Result<(), ModelProviderProblem> {
    match input {
        ModelProviderInput::Preset { id, key } => {
            let path = ["models", "providers", id.as_str(), "test"];
            api.post_unit_detailed(&path, &ModelProviderKey { key }, SOURCE_TIMEOUT)
                .await
        }
        ModelProviderInput::Custom {
            base_url,
            model,
            key,
        } => {
            let body = CustomModelProvider {
                base_url,
                model,
                key,
            };
            api.post_unit_detailed(
                &["models", "custom-provider", "test"],
                &body,
                SOURCE_TIMEOUT,
            )
            .await
        }
    }
    .map_err(map_error)
}

pub(crate) async fn save_provider(
    api: &CoreApi,
    input: ModelProviderInput,
) -> Result<(), ModelProviderProblem> {
    match input {
        ModelProviderInput::Preset { id, key } => {
            let path = ["models", "providers", id.as_str()];
            api.put_unit_detailed(&path, &ModelProviderKey { key }, SOURCE_TIMEOUT)
                .await
        }
        ModelProviderInput::Custom {
            base_url,
            model,
            key,
        } => {
            let body = CustomModelProvider {
                base_url,
                model,
                key,
            };
            api.put_unit_detailed(&["models", "custom-provider"], &body, SOURCE_TIMEOUT)
                .await
        }
    }
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
