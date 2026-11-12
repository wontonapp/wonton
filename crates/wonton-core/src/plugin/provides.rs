//! 插件给其他插件提供的能力（`provides`）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::languages::Languages;
use super::manifest::{Manifest, ManifestError, Permission, PluginKind};
use crate::l10n::text;

/// 插件给其他插件提供的能力（ADR-0053）：`models` 和 `egress_proxy` 有专用字段，其余能力（核心定义的
/// 或第三方的）按能力名放进 [`Self::capabilities`]。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<ModelsProvider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress_proxy: Option<EgressProxyProvider>,
    /// 其他能力的提供方，键是能力名：核心定义的（如 `search`）或第三方的（如 `x-acme.ledger`）。
    #[serde(default, flatten)]
    pub capabilities: BTreeMap<String, ProviderService>,
}

/// 一个能力提供方监听的服务和端口，可选的凭据命令（ADR-0053）。服务名是否存在由 [`super::package`]
/// 对照 compose 检查。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderService {
    pub service: String,
    pub port: u16,
    /// 声明了就给每个使用方签一个只属于它的凭据，注入 `WONTON_<名字>_KEY`；不声明时提供方只认经核心
    /// 转发来的请求（ADR-0053）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<CredentialCommands>,
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
pub fn validate(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    let egress = manifest
        .provides
        .as_ref()
        .and_then(|p| p.egress_proxy.as_ref());
    let permission = manifest.permissions.contains(&Permission::EgressProxy);
    match (egress, permission) {
        (Some(_), false) => {
            return Err(invalid(&text(
                langs,
                "plugin-install-prv-egress-needs-permission",
                &[],
            )));
        }
        (None, true) => {
            return Err(invalid(&text(
                langs,
                "plugin-install-prv-permission-needs-egress",
                &[],
            )));
        }
        _ => {}
    }
    if let Some(egress) = egress {
        if !manifest.permissions.contains(&Permission::NetworkEgress) {
            return Err(invalid(&text(
                langs,
                "plugin-install-prv-egress-needs-network",
                &[],
            )));
        }
        if egress.port == 0 {
            return Err(invalid(&text(langs, "plugin-install-prv-egress-port", &[])));
        }
    }
    let Some(provides) = &manifest.provides else {
        return Ok(());
    };
    if provides.models.is_none()
        && provides.egress_proxy.is_none()
        && provides.capabilities.is_empty()
    {
        return Err(invalid(&text(langs, "plugin-install-prv-empty", &[])));
    }
    let Some(models) = &provides.models else {
        return Ok(());
    };
    if manifest.kind != PluginKind::System {
        return Err(invalid(&text(
            langs,
            "plugin-install-prv-models-system-only",
            &[],
        )));
    }
    validate_models(models, langs)
}

fn validate_models(models: &ModelsProvider, langs: &Languages) -> Result<(), ManifestError> {
    if models.port == 0 {
        return Err(invalid(&text(langs, "plugin-install-prv-models-port", &[])));
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
        return Err(invalid(&text(
            langs,
            "plugin-install-prv-models-apis-path",
            &[],
        )));
    }
    if models.default_model.trim().is_empty() {
        return Err(invalid(&text(
            langs,
            "plugin-install-prv-models-default-model",
            &[],
        )));
    }
    for (field, argv) in [
        ("issue", &models.credentials.issue),
        ("revoke", &models.credentials.revoke),
    ] {
        if !argv.iter().any(|arg| arg.contains(NAME_PLACEHOLDER)) {
            return Err(invalid(&text(
                langs,
                "plugin-install-prv-credentials-argv",
                &[
                    ("field", field.into()),
                    ("placeholder", NAME_PLACEHOLDER.into()),
                ],
            )));
        }
    }
    if let Some(providers) = &models.providers {
        validate_provider_commands(providers, langs)?;
    }
    Ok(())
}

fn validate_provider_commands(
    providers: &ProviderCommands,
    langs: &Languages,
) -> Result<(), ManifestError> {
    if providers.list.is_empty() {
        return Err(invalid(&text(langs, "plugin-install-prv-list-argv", &[])));
    }
    for (field, argv) in [("save", &providers.save), ("test", &providers.test)] {
        if !argv.iter().any(|arg| arg.contains(ID_PLACEHOLDER)) {
            return Err(invalid(&text(
                langs,
                "plugin-install-prv-provider-argv",
                &[("field", field.into())],
            )));
        }
    }
    if let Some(custom) = &providers.custom {
        for (field, argv) in [("save", &custom.save), ("test", &custom.test)] {
            if argv.is_empty() || argv.iter().any(|arg| arg.contains(['{', '}'])) {
                return Err(invalid(&text(
                    langs,
                    "plugin-install-prv-custom-argv",
                    &[("field", field.into())],
                )));
            }
        }
    }
    Ok(())
}
