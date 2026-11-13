//! 模型网关的服务商列表、key 测试和保存命令。

use std::collections::BTreeSet;
use std::time::Duration;

use serde::Deserialize;
use wonton_protocol::{
    ConfiguredModelProvider, CustomModelProvider, ModelProvider, ModelProviders,
};

use super::PluginId;
use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::models::{COMMAND_TIMEOUT, ModelsRoute, ProviderState};
use super::provides::{ID_PLACEHOLDER, ProviderCommands};
use super::storefront::onboarding::OnboardingCatalog;

const PROVIDER_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_KEY_BYTES: usize = 4 << 10;

#[derive(Clone, Copy, Debug)]
pub(crate) enum ProviderAction {
    Save,
    Test,
}

impl ProviderAction {
    fn name(self, langs: &Languages) -> String {
        match self {
            Self::Save => crate::l10n::text(langs, "plugin-settings-model-action-save", &[]),
            Self::Test => crate::l10n::text(langs, "plugin-settings-model-action-test", &[]),
        }
    }

    fn disconnected(self, langs: &Languages) -> String {
        match self {
            Self::Save => crate::l10n::text(langs, "plugin-settings-model-disconnected-saved", &[]),
            Self::Test => {
                crate::l10n::text(langs, "plugin-settings-model-disconnected-unsaved", &[])
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProviderList {
    presets: Vec<ProviderPreset>,
    configured: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ProviderPreset {
    id: String,
    title: String,
}

impl PluginManager {
    /// 网关插件的网页地址，没有网页时为空。
    pub(crate) fn model_web_url(&self, id: &PluginId) -> Option<String> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins.get(id).and_then(|entry| self.open_url(id, entry))
    }

    /// 列出官方索引允许且模型网关支持的服务商。
    pub(crate) async fn model_providers(
        &self,
        langs: &Languages,
    ) -> Result<ModelProviders, InstallError> {
        let route = self.provider_route(langs)?;
        let Some(commands) = route.spec.providers.as_ref() else {
            return Ok(ModelProviders {
                plugin_id: route.id.to_string(),
                providers: None,
                any_configured: false,
                web_url: self.model_web_url(&route.id),
                configured: Vec::new(),
                custom: false,
            });
        };
        let listed = self.list_providers(&route, commands, langs).await?;
        let (index, _) = self.official_index().await;
        let any_configured = !listed.configured.is_empty();
        let configured = configured_providers(&listed);
        let providers = merge_providers(listed, index.onboarding.as_ref(), langs);
        Ok(ModelProviders {
            plugin_id: route.id.to_string(),
            providers: Some(providers),
            any_configured,
            web_url: self.model_web_url(&route.id),
            configured,
            custom: commands.custom.is_some(),
        })
    }

    pub(crate) async fn model_provider_key(
        &self,
        id: &str,
        key: &str,
        action: ProviderAction,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        validate_key(key, langs)?;
        let route = self.provider_route(langs)?;
        let commands = route.spec.providers.as_ref().ok_or_else(|| {
            unavailable(crate::l10n::text(
                langs,
                "plugin-settings-model-no-provider-commands",
                &[("provider", route.id.as_str().into())],
            ))
        })?;
        let listed = self.list_providers(&route, commands, langs).await?;
        // 约定是 configured 的每一项都在 presets 里（脚本的 list 负责补齐，核心不认识
        // magpie"已添加的 vendor 从 presets 表消失"的行为，见 ADR-0015）。
        if !listed.presets.iter().any(|preset| preset.id == id) {
            return Err(InstallError::Invalid(crate::l10n::text(
                langs,
                "plugin-settings-model-provider-unsupported",
                &[("id", id.into())],
            )));
        }
        let argv = match action {
            ProviderAction::Save => &commands.save,
            ProviderAction::Test => &commands.test,
        };
        let argv = replace(argv, ID_PLACEHOLDER, id);
        let stdin = format!("{key}\n");
        self.run_provider_command(&route, &argv, stdin.as_bytes(), action, false, langs)
            .await
    }

    pub(crate) async fn custom_model_provider(
        &self,
        provider: &CustomModelProvider,
        action: ProviderAction,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        validate_custom(provider, langs)?;
        let route = self.provider_route(langs)?;
        let custom = route
            .spec
            .providers
            .as_ref()
            .and_then(|commands| commands.custom.as_ref())
            .ok_or_else(|| {
                unavailable(crate::l10n::text(
                    langs,
                    "plugin-settings-model-custom-unsupported",
                    &[],
                ))
            })?;
        let argv = match action {
            ProviderAction::Save => &custom.save,
            ProviderAction::Test => &custom.test,
        };
        let mut stdin = serde_json::to_vec(provider).map_err(|_| {
            unavailable(crate::l10n::text(
                langs,
                "plugin-settings-model-custom-encode-failed",
                &[],
            ))
        })?;
        stdin.push(b'\n');
        self.run_provider_command(&route, argv, &stdin, action, true, langs)
            .await
    }

    async fn run_provider_command(
        &self,
        route: &ModelsRoute,
        argv: &[String],
        stdin: &[u8],
        action: ProviderAction,
        custom: bool,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let output = self
            .engine
            .exec(
                &route.credentials_container,
                argv,
                Some(stdin),
                PROVIDER_COMMAND_TIMEOUT,
            )
            .await
            .map_err(|_| {
                unavailable(crate::l10n::text(
                    langs,
                    "plugin-settings-model-command-timeout",
                    &[("provider", route.id.as_str().into())],
                ))
            })?;
        tracing::info!(action = %action.name(langs), exit_code = output.exit_code, "模型服务商命令结束");
        match output.exit_code {
            0 => Ok(()),
            2 if custom => Err(InstallError::Invalid(crate::l10n::text(
                langs,
                "plugin-settings-model-invalid-endpoint",
                &[],
            ))),
            3 => Err(InstallError::Invalid(crate::l10n::text(
                langs,
                "plugin-settings-model-key-rejected",
                &[],
            ))),
            4 => Err(InstallError::UpstreamUnavailable(
                action.disconnected(langs),
            )),
            code => Err(unavailable(crate::l10n::text(
                langs,
                "plugin-settings-model-provider-command-failed",
                &[
                    ("provider", route.id.as_str().into()),
                    ("action", action.name(langs).into()),
                    ("code", code.to_string().into()),
                ],
            ))),
        }
    }

    fn provider_route(&self, langs: &Languages) -> Result<ModelsRoute, InstallError> {
        match self.models_provider() {
            ProviderState::Ready(route) => Ok(*route),
            ProviderState::NotReady(id) => Err(unavailable(super::models::gateway_not_running(
                langs,
                id.as_str(),
            ))),
            ProviderState::Absent => Err(unavailable(super::models::gateway_absent(langs))),
        }
    }

    async fn list_providers(
        &self,
        route: &ModelsRoute,
        commands: &ProviderCommands,
        langs: &Languages,
    ) -> Result<ProviderList, InstallError> {
        let output = self
            .engine
            .exec_output(
                &route.credentials_container,
                &commands.list,
                None,
                COMMAND_TIMEOUT,
            )
            .await
            .map_err(|e| {
                unavailable(crate::l10n::text(
                    langs,
                    "plugin-settings-model-list-failed",
                    &[
                        ("provider", route.id.as_str().into()),
                        ("error", e.to_string().into()),
                    ],
                ))
            })?;
        if output.exit_code != 0 {
            return Err(unavailable(crate::l10n::text(
                langs,
                "plugin-settings-model-list-exit",
                &[
                    ("provider", route.id.as_str().into()),
                    ("code", output.exit_code.to_string().into()),
                ],
            )));
        }
        serde_json::from_str(&output.stdout).map_err(|_| {
            unavailable(crate::l10n::text(
                langs,
                "plugin-settings-model-list-not-json",
                &[("provider", route.id.as_str().into())],
            ))
        })
    }
}

fn unavailable(message: impl Into<String>) -> InstallError {
    InstallError::ModelsUnavailable(message.into())
}

fn validate_key(key: &str, langs: &Languages) -> Result<(), InstallError> {
    if key.is_empty() || key.contains('\n') || key.contains('\r') || key.len() > MAX_KEY_BYTES {
        return Err(InstallError::Invalid(crate::l10n::text(
            langs,
            "plugin-settings-model-key-invalid",
            &[],
        )));
    }
    Ok(())
}

fn validate_custom(provider: &CustomModelProvider, langs: &Languages) -> Result<(), InstallError> {
    validate_key(provider.key.expose(), langs)?;
    if provider.base_url.len() > 2 << 10
        || provider.base_url.chars().any(char::is_whitespace)
        || !url::Url::parse(&provider.base_url)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
    {
        return Err(InstallError::Invalid(crate::l10n::text(
            langs,
            "plugin-settings-model-url-invalid",
            &[],
        )));
    }
    if provider.model.is_empty()
        || provider.model.len() > 200
        || provider
            .model
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(InstallError::Invalid(crate::l10n::text(
            langs,
            "plugin-settings-model-name-invalid",
            &[],
        )));
    }
    Ok(())
}

fn replace(args: &[String], placeholder: &str, value: &str) -> Vec<String> {
    args.iter()
        .map(|arg| arg.replace(placeholder, value))
        .collect()
}

/// 网关里已经接好的，按网关的顺序；名字取 `presets` 里的，没有时用 ID。
fn configured_providers(listed: &ProviderList) -> Vec<ConfiguredModelProvider> {
    listed
        .configured
        .iter()
        .map(|id| ConfiguredModelProvider {
            id: id.clone(),
            title: listed
                .presets
                .iter()
                .find(|preset| preset.id == *id)
                .map_or_else(|| id.clone(), |preset| preset.title.clone()),
        })
        .collect()
}

fn merge_providers(
    listed: ProviderList,
    catalog: Option<&OnboardingCatalog>,
    langs: &Languages,
) -> Vec<ModelProvider> {
    let Some(catalog) = catalog else {
        return Vec::new();
    };
    let recommendations = langs
        .pick_map(&catalog.providers)
        .into_iter()
        .flatten()
        .map(|provider| (provider, true));
    let more = catalog
        .more_providers
        .iter()
        .map(|provider| (provider, false));
    let configured: BTreeSet<String> = listed.configured.into_iter().collect();
    let mut seen = BTreeSet::new();
    let mut providers = Vec::new();
    for (provider, recommended) in recommendations.chain(more) {
        let Some(preset) = listed.presets.iter().find(|p| p.id == provider.id) else {
            continue;
        };
        if seen.insert(&preset.id) {
            providers.push(ModelProvider {
                id: preset.id.clone(),
                title: preset.title.clone(),
                key_url: Some(provider.key_url.clone()),
                recommended,
                configured: configured.contains(&preset.id),
            });
        }
    }
    providers
}
