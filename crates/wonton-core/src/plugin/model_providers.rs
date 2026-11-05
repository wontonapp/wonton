//! 模型网关的服务商列表和 key 保存命令。

use std::collections::BTreeSet;
use std::time::Duration;

use serde::Deserialize;
use wonton_protocol::{ModelProvider, ModelProviders};

use super::PluginId;
use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::models::{COMMAND_TIMEOUT, ModelsRoute, ProviderState};
use super::provides::{ID_PLACEHOLDER, ProviderCommands};
use super::storefront::onboarding::OnboardingProvider;

const SAVE_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_KEY_BYTES: usize = 4 << 10;

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
    /// 网关插件的网页地址（和已安装列表里给插件 webui 的同一个算法），没有网页时为空。
    pub(crate) fn model_web_url(&self, id: &PluginId) -> Option<String> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .get(id)
            .and_then(|entry| entry.has_webui.then(|| self.web_url(id)))
    }

    /// 列出模型网关支持的服务商，并按官方索引合并推荐项。
    pub(crate) async fn model_providers(
        &self,
        langs: &Languages,
    ) -> Result<ModelProviders, InstallError> {
        let route = self.provider_route()?;
        let Some(commands) = route.spec.providers.as_ref() else {
            return Ok(ModelProviders {
                providers: None,
                web_url: self.model_web_url(&route.id),
            });
        };
        let listed = self.list_providers(&route, commands).await?;
        let recommendations = self.provider_recommendations(langs).await;
        let providers = merge_providers(listed, &recommendations);
        Ok(ModelProviders {
            providers: Some(providers),
            web_url: self.model_web_url(&route.id),
        })
    }

    /// 验证并保存一个服务商 key。key 只经标准输入传给网关命令。
    pub(crate) async fn save_model_provider(
        &self,
        id: &str,
        key: &str,
    ) -> Result<(), InstallError> {
        let route = self.provider_route()?;
        let commands = route
            .spec
            .providers
            .as_ref()
            .ok_or_else(|| unavailable(format!("模型网关 {} 没有声明服务商命令", route.id)))?;
        let listed = self.list_providers(&route, commands).await?;
        // 约定是 configured 的每一项都在 presets 里（脚本的 list 负责补齐，核心不认识
        // magpie"已添加的 vendor 从 presets 表消失"的行为，见 ADR-0015）。
        if !listed.presets.iter().any(|preset| preset.id == id) {
            return Err(InstallError::Invalid(format!("模型网关不支持服务商 {id}")));
        }
        validate_key(key)?;
        let argv = replace(commands.save.as_slice(), ID_PLACEHOLDER, id);
        let stdin = format!("{key}\n");
        let output = self
            .engine
            .exec(
                &route.credentials_container,
                &argv,
                Some(stdin.as_bytes()),
                SAVE_COMMAND_TIMEOUT,
            )
            .await
            .map_err(|e| unavailable(format!("模型网关 {} 未就绪：{e}", route.id)))?;
        tracing::info!(
            provider = id,
            exit_code = output.exit_code,
            "模型服务商保存命令结束"
        );
        match output.exit_code {
            0 => Ok(()),
            3 => Err(InstallError::Invalid(format!(
                "服务商 {id} 拒绝了 key，key 没有保存"
            ))),
            4 => Err(InstallError::UpstreamUnavailable(format!(
                "服务商 {id} 连接失败，key 已保存"
            ))),
            code => Err(unavailable(format!(
                "模型网关 {} 保存服务商 {id} 失败，退出码 {code}",
                route.id
            ))),
        }
    }

    fn provider_route(&self) -> Result<ModelsRoute, InstallError> {
        match self.models_provider() {
            ProviderState::Ready(route) => Ok(*route),
            ProviderState::NotReady(id) => Err(unavailable(format!("模型网关 {id} 没在运行"))),
            ProviderState::Absent => Err(unavailable(
                "没有装模型网关（声明 provides.models 的系统插件）",
            )),
        }
    }

    async fn list_providers(
        &self,
        route: &ModelsRoute,
        commands: &ProviderCommands,
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
            .map_err(|e| unavailable(format!("模型网关 {} 列出服务商失败：{e}", route.id)))?;
        if output.exit_code != 0 {
            return Err(unavailable(format!(
                "模型网关 {} 列出服务商失败，退出码 {}：{}",
                route.id, output.exit_code, output.stderr
            )));
        }
        serde_json::from_str(&output.stdout).map_err(|e| {
            unavailable(format!(
                "模型网关 {} 的服务商列表不是合法 JSON：{e}",
                route.id
            ))
        })
    }

    async fn provider_recommendations(&self, langs: &Languages) -> Vec<OnboardingProvider> {
        let (index, _) = self.official_index().await;
        index
            .onboarding
            .as_ref()
            .and_then(|catalog| langs.pick_map(&catalog.providers))
            .cloned()
            .unwrap_or_default()
    }
}

fn unavailable(message: impl Into<String>) -> InstallError {
    InstallError::ModelsUnavailable(message.into())
}

fn validate_key(key: &str) -> Result<(), InstallError> {
    if key.is_empty() || key.contains('\n') || key.contains('\r') || key.len() > MAX_KEY_BYTES {
        return Err(InstallError::Invalid(
            "key 不能为空、不能有换行，且不能超过 4 KiB".to_owned(),
        ));
    }
    Ok(())
}

fn replace(args: &[String], placeholder: &str, value: &str) -> Vec<String> {
    args.iter()
        .map(|arg| arg.replace(placeholder, value))
        .collect()
}

fn merge_providers(
    listed: ProviderList,
    recommendations: &[OnboardingProvider],
) -> Vec<ModelProvider> {
    let configured: BTreeSet<String> = listed.configured.into_iter().collect();
    let mut recommended = BTreeSet::new();
    let mut providers = Vec::new();
    for recommendation in recommendations {
        let Some(preset) = listed.presets.iter().find(|p| p.id == recommendation.id) else {
            continue;
        };
        if recommended.insert(preset.id.clone()) {
            providers.push(model_provider(
                preset,
                &configured,
                Some(recommendation.key_url.clone()),
                true,
            ));
        }
    }
    for preset in &listed.presets {
        if !recommended.contains(&preset.id) {
            providers.push(model_provider(preset, &configured, None, false));
        }
    }
    providers
}

fn model_provider(
    preset: &ProviderPreset,
    configured: &BTreeSet<String>,
    key_url: Option<String>,
    recommended: bool,
) -> ModelProvider {
    ModelProvider {
        id: preset.id.clone(),
        title: preset.title.clone(),
        key_url,
        recommended,
        configured: configured.contains(&preset.id),
    }
}
