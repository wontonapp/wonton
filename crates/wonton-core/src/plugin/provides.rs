//! 插件给其他插件提供的能力（`provides`）。

use serde::{Deserialize, Serialize};

use super::manifest::{Manifest, ManifestError, Permission, PluginKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provides {
    #[serde(default)]
    pub models: Option<ModelsProvider>,
    #[serde(default)]
    pub egress_proxy: Option<EgressProxyProvider>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EgressProxyProvider {
    pub service: String,
    pub port: u16,
}

/// 模型网关：声明了 `models` 的插件经核心转发访问它，各自拿一个只属于自己的凭据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelsProvider {
    /// 网关监听的服务和端口，核心只转发到这里。
    pub service: String,
    pub port: u16,
    pub apis: ModelApis,
    /// 注入给使用方的 `WONTON_MODEL_DEFAULT`。
    pub default_model: String,
    pub credentials: CredentialCommands,
    /// 服务商管理命令，在 credentials 容器里执行。
    #[serde(default)]
    pub providers: Option<ProviderCommands>,
}

/// 服务商命令（argv，不经 shell）。预设命令用 `{id}` 指明服务商 ID。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCommands {
    pub list: Vec<String>,
    pub save: Vec<String>,
    pub test: Vec<String>,
    #[serde(default)]
    pub custom: Option<CustomProviderCommands>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomProviderCommands {
    pub save: Vec<String>,
    pub test: Vec<String>,
}

/// 网关提供的接口及其基础路径。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelApis {
    pub openai: String,
    #[serde(default)]
    pub anthropic: Option<String>,
    #[serde(default)]
    pub gemini: Option<String>,
}

/// 签发和吊销凭据的命令（argv，不经 shell），`{name}` 换成凭据名。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialCommands {
    pub issue: Vec<String>,
    pub revoke: Vec<String>,
    /// 在哪个服务的容器里执行，默认是 `service`。
    #[serde(default)]
    pub service: Option<String>,
}

/// 服务商命令里的服务商 ID 占位符。
pub const ID_PLACEHOLDER: &str = "{id}";

/// 凭据命令里的凭据名占位符。
pub const NAME_PLACEHOLDER: &str = "{name}";

impl ModelsProvider {
    /// 执行凭据命令的服务。
    pub fn credentials_service(&self) -> &str {
        self.credentials.service.as_deref().unwrap_or(&self.service)
    }
}

fn invalid(message: &str) -> ManifestError {
    ManifestError::Invalid(message.to_owned())
}

/// 服务名是否存在由 [`super::package`] 对照 compose 检查。
pub fn validate(manifest: &Manifest) -> Result<(), ManifestError> {
    let egress = manifest
        .provides
        .as_ref()
        .and_then(|p| p.egress_proxy.as_ref());
    let permission = manifest.permissions.contains(&Permission::EgressProxy);
    match (egress, permission) {
        (Some(_), false) => {
            return Err(invalid("provides.egress_proxy 必须申请 egress.proxy 权限"));
        }
        (None, true) => return Err(invalid("egress.proxy 权限必须声明 provides.egress_proxy")),
        _ => {}
    }
    if let Some(egress) = egress {
        if !manifest.permissions.contains(&Permission::NetworkEgress) {
            return Err(invalid(
                "provides.egress_proxy 必须申请 network.egress 权限",
            ));
        }
        if egress.port == 0 {
            return Err(invalid("provides.egress_proxy.port 不能为 0"));
        }
    }
    let Some(provides) = &manifest.provides else {
        return Ok(());
    };
    if provides.models.is_none() && provides.egress_proxy.is_none() {
        return Err(invalid("provides 至少声明一项能力"));
    }
    let Some(models) = &provides.models else {
        return Ok(());
    };
    if manifest.kind != PluginKind::System {
        return Err(invalid("只有 kind: system 的插件能声明 provides.models"));
    }
    validate_models(models)
}

fn validate_models(models: &ModelsProvider) -> Result<(), ManifestError> {
    if models.port == 0 {
        return Err(invalid("provides.models.port 不能为 0"));
    }
    let apis = [
        Some(&models.apis.openai),
        models.apis.anthropic.as_ref(),
        models.apis.gemini.as_ref(),
    ];
    if apis
        .into_iter()
        .flatten()
        .any(|path| !path.starts_with('/'))
    {
        return Err(invalid("provides.models.apis 的路径必须以 / 开头"));
    }
    if models.default_model.trim().is_empty() {
        return Err(invalid("provides.models.default_model 不能为空"));
    }
    for (field, argv) in [
        ("issue", &models.credentials.issue),
        ("revoke", &models.credentials.revoke),
    ] {
        if !argv.iter().any(|arg| arg.contains(NAME_PLACEHOLDER)) {
            return Err(ManifestError::Invalid(format!(
                "provides.models.credentials.{field} 是非空的 argv，并用 {NAME_PLACEHOLDER} 指明凭据名"
            )));
        }
    }
    if let Some(providers) = &models.providers {
        validate_provider_commands(providers)?;
    }
    Ok(())
}

fn validate_provider_commands(providers: &ProviderCommands) -> Result<(), ManifestError> {
    if providers.list.is_empty() {
        return Err(invalid("provides.models.providers.list 必须是非空的 argv"));
    }
    for (field, argv) in [("save", &providers.save), ("test", &providers.test)] {
        if !argv.iter().any(|arg| arg.contains(ID_PLACEHOLDER)) {
            return Err(invalid(&format!(
                "provides.models.providers.{field} 是非空的 argv，并用 {{id}} 指明服务商 ID"
            )));
        }
    }
    if let Some(custom) = &providers.custom {
        for (field, argv) in [("save", &custom.save), ("test", &custom.test)] {
            if argv.is_empty() || argv.iter().any(|arg| arg.contains(['{', '}'])) {
                return Err(invalid(&format!(
                    "provides.models.providers.custom.{field} 必须是非空且不含占位符的 argv"
                )));
            }
        }
    }
    Ok(())
}
