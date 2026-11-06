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
    fn name(self) -> &'static str {
        match self {
            Self::Save => "保存",
            Self::Test => "测试",
        }
    }

    fn disconnected(self) -> &'static str {
        match self {
            Self::Save => "服务商连接失败，key 已保存",
            Self::Test => "服务商连接失败，什么都没保存",
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
    /// 网关插件的网页地址（和已安装列表里给插件 webui 的同一个算法），没有网页时为空。
    pub(crate) fn model_web_url(&self, id: &PluginId) -> Option<String> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .get(id)
            .and_then(|entry| entry.has_webui.then(|| self.web_url(id)))
    }

    /// 列出官方索引允许且模型网关支持的服务商。
    pub(crate) async fn model_providers(
        &self,
        langs: &Languages,
    ) -> Result<ModelProviders, InstallError> {
        let route = self.provider_route()?;
        let Some(commands) = route.spec.providers.as_ref() else {
            return Ok(ModelProviders {
                providers: None,
                any_configured: false,
                web_url: self.model_web_url(&route.id),
                configured: Vec::new(),
                custom: false,
            });
        };
        let listed = self.list_providers(&route, commands).await?;
        let (index, _) = self.official_index().await;
        let any_configured = !listed.configured.is_empty();
        let configured = configured_providers(&listed);
        let providers = merge_providers(listed, index.onboarding.as_ref(), langs);
        Ok(ModelProviders {
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
    ) -> Result<(), InstallError> {
        validate_key(key)?;
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
        let argv = match action {
            ProviderAction::Save => &commands.save,
            ProviderAction::Test => &commands.test,
        };
        let argv = replace(argv, ID_PLACEHOLDER, id);
        let stdin = format!("{key}\n");
        self.run_provider_command(&route, &argv, stdin.as_bytes(), action, false)
            .await
    }

    pub(crate) async fn custom_model_provider(
        &self,
        provider: &CustomModelProvider,
        action: ProviderAction,
    ) -> Result<(), InstallError> {
        validate_custom(provider)?;
        let route = self.provider_route()?;
        let custom = route
            .spec
            .providers
            .as_ref()
            .and_then(|commands| commands.custom.as_ref())
            .ok_or_else(|| unavailable("模型网关不支持自定义服务商"))?;
        let argv = match action {
            ProviderAction::Save => &custom.save,
            ProviderAction::Test => &custom.test,
        };
        let mut stdin =
            serde_json::to_vec(provider).map_err(|_| unavailable("自定义服务商请求编码失败"))?;
        stdin.push(b'\n');
        self.run_provider_command(&route, argv, &stdin, action, true)
            .await
    }

    async fn run_provider_command(
        &self,
        route: &ModelsRoute,
        argv: &[String],
        stdin: &[u8],
        action: ProviderAction,
        custom: bool,
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
            .map_err(|_| unavailable(format!("模型网关 {} 命令执行失败或超时", route.id)))?;
        tracing::info!(
            action = action.name(),
            exit_code = output.exit_code,
            "模型服务商命令结束"
        );
        match output.exit_code {
            0 => Ok(()),
            2 if custom => Err(InstallError::Invalid("服务商地址或模型名不合法".into())),
            3 => Err(InstallError::Invalid(
                "服务商拒绝了 key，key 没有保存".into(),
            )),
            4 => Err(InstallError::UpstreamUnavailable(
                action.disconnected().into(),
            )),
            code => Err(unavailable(format!(
                "模型网关 {} {}服务商失败，退出码 {code}",
                route.id,
                action.name()
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
                "模型网关 {} 列出服务商失败，退出码 {}",
                route.id, output.exit_code
            )));
        }
        serde_json::from_str(&output.stdout)
            .map_err(|_| unavailable(format!("模型网关 {} 的服务商列表不是合法 JSON", route.id)))
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

fn validate_custom(provider: &CustomModelProvider) -> Result<(), InstallError> {
    validate_key(provider.key.expose())?;
    if provider.base_url.len() > 2 << 10
        || provider.base_url.chars().any(char::is_whitespace)
        || !url::Url::parse(&provider.base_url)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
    {
        return Err(InstallError::Invalid(
            "服务商地址必须是 http 或 https URL、不能有空白，且不能超过 2 KiB".into(),
        ));
    }
    if provider.model.is_empty()
        || provider.model.len() > 200
        || provider
            .model
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(InstallError::Invalid(
            "模型名不能为空、不能有空白或控制字符，且不能超过 200 字节".into(),
        ));
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
