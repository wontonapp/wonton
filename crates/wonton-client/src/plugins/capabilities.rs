//! 能力依赖的客户端动作（ADR-0053）：预览里给缺的必需能力选提供方、给已装插件换提供方、卸载前问
//! 核心，以及详情页依赖异常的下一步。

use wonton_l10n::l10n;
use wonton_protocol::{
    CapabilityProvider, ConsumerCapabilityChoice, CoreError, DependencyNextStep, InstallPreview,
    InstallProvider, InstallSource, PluginDetail, PluginDetailRoute, PluginHint,
    PluginHintCategory, SelectedProvider, UninstallOutcome, UninstallPreview,
};

use super::PluginsLink;
use super::api::CoreApiError;
use super::api::{READ_TIMEOUT, SOURCE_TIMEOUT, to_core_error};
use crate::l10n::{CATALOG, language};

impl PluginsLink {
    /// 预览里缺的必需能力各选索引里的第一个提供方（和以前安装时的自动选择同一条规则），取它自己的
    /// 完整预览放进 `install_providers`；商店里没有提供方的能力留一项空的，确认页据此说装不上。
    pub(super) async fn install_providers(
        &self,
        preview: &mut InstallPreview,
    ) -> Result<(), CoreError> {
        let mut sections = Vec::with_capacity(preview.missing_requires.len());
        // 同一个提供方补几个能力时只取一次预览（和安装时只装它一次对上）。
        let mut loaded: std::collections::HashMap<String, InstallPreview> = Default::default();
        for missing in &preview.missing_requires {
            let selected = match missing.providers.first() {
                Some(provider) => {
                    let preview = match loaded.get(&provider.id) {
                        Some(preview) => preview.clone(),
                        None => {
                            let preview = self.provider_preview(provider).await?;
                            loaded.insert(provider.id.clone(), preview.clone());
                            preview
                        }
                    };
                    Some(SelectedProvider {
                        provider: provider.clone(),
                        preview,
                        secrets: Default::default(),
                    })
                }
                None => None,
            };
            sections.push(InstallProvider {
                capability: missing.capability.clone(),
                third_party: missing.third_party,
                providers: missing.providers.clone(),
                selected,
            });
        }
        preview.install_providers = sections;
        Ok(())
    }

    /// 按提供方的来源取它自己的完整预览。回来的 ID 对不上是来源换了，别装错。
    async fn provider_preview(
        &self,
        provider: &CapabilityProvider,
    ) -> Result<InstallPreview, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let preview: InstallPreview = api
            .post(
                &["plugins", "preview"],
                &InstallSource {
                    source: provider.source.clone(),
                },
                SOURCE_TIMEOUT,
            )
            .await
            .map_err(to_core_error)?;
        if preview.id != provider.id {
            return Err(CoreError::InvalidInput {
                reason: l10n!(
                    CATALOG,
                    language(),
                    "plugins-capability-wrong-provider",
                    provider = provider.id.as_str(),
                    got = preview.id.as_str()
                ),
            });
        }
        Ok(preview)
    }

    /// 给这个插件的一项能力选提供方；`provider` 为空表示跟全局默认走。返回重新取的详情。
    pub(crate) async fn set_capability(
        &self,
        id: &str,
        capability: &str,
        provider: Option<String>,
    ) -> Result<PluginDetail, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.put_unit_body(
            &["plugins", id, "capabilities"],
            &ConsumerCapabilityChoice {
                capability: capability.to_owned(),
                provider,
            },
            READ_TIMEOUT,
        )
        .await
        .map_err(to_core_error)?;
        self.fetch_detail(&PluginDetailRoute::Installed { id: id.to_owned() })
            .await
            .map_err(to_core_error)
    }

    /// 卸载面板打开先问核心：能卸时带上没人再用的提供方，被在用的依赖挡住时带上依赖方。
    pub(crate) async fn uninstall_preview(&self, id: &str) -> Result<UninstallOutcome, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let preview = api
            .get_detailed::<UninstallPreview>(
                &["plugins", id, "uninstall", "preview"],
                &[],
                READ_TIMEOUT,
            )
            .await;
        match preview {
            Ok(preview) => Ok(UninstallOutcome::Ready {
                unused_providers: preview.unused_providers,
            }),
            Err(CoreApiError::Http {
                kind: Some(kind),
                plugins,
                ..
            }) if kind == "in_use" => Ok(UninstallOutcome::InUse {
                dependents: plugins,
            }),
            Err(error) => Err(to_core_error(error.into_problem())),
        }
    }
}

/// 这类提示给主人的下一步：启动提供方、接上提供方、去商店找提供方。别的类别没有下一步。
#[uniffi::export]
pub fn dependency_next_step(hint: PluginHint) -> Option<DependencyNextStep> {
    match hint.category {
        PluginHintCategory::DependencyStopped if hint.provider.is_some() => {
            Some(DependencyNextStep::StartProvider)
        }
        PluginHintCategory::ProviderNotConnected
            if hint.provider.is_some() && hint.capability.is_some() =>
        {
            Some(DependencyNextStep::ConnectProvider)
        }
        PluginHintCategory::WantsUnsatisfied => Some(DependencyNextStep::OpenStore),
        _ => None,
    }
}
