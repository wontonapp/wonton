use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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

/// 一个能力提供方监听的服务和端口，可选的凭据命令（ADR-0053）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderService {
    pub service: String,
    pub port: u16,
    /// 声明了就给每个使用方签一个只属于它的凭据，注入 `WONTON_<名字>_KEY`；不声明时提供方只认经核心
    /// 转发来的请求（ADR-0053）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<CredentialCommands>,
    /// 托管能力（ADR-0080）：核心把这项能力的总目录挂到容器里的这个绝对路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount: Option<String>,
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

impl ModelsProvider {
    /// 执行凭据命令的服务。
    pub fn credentials_service(&self) -> &str {
        self.credentials.service.as_deref().unwrap_or(&self.service)
    }
}
