//! 系统插件给其他插件提供的能力（`provides`）。M1 只有模型网关（`provides.models`，插件规范的“模型网关”）。

use serde::{Deserialize, Serialize};

use super::manifest::{Manifest, ManifestError, PluginKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provides {
    #[serde(default)]
    pub models: Option<ModelsProvider>,
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

/// 只有 `kind: system` 的插件能声明 `provides`：提供方看得到全部请求和 provider 的 key。服务名是否存在由
/// [`super::package`] 对照 compose 检查。
pub fn validate(manifest: &Manifest) -> Result<(), ManifestError> {
    let Some(provides) = &manifest.provides else {
        return Ok(());
    };
    if manifest.kind != PluginKind::System {
        return Err(invalid("只有 kind: system 的插件能声明 provides"));
    }
    let Some(models) = &provides.models else {
        return Err(invalid("provides 至少声明一项能力"));
    };
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
    Ok(())
}
