use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::media::{MediaKind, MediaOperation};

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
    /// 自报能做的操作；核心仍以 `/_wonton/capabilities` 为准，这里给安装预检。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<MediaOperation>,
    /// 自报的内容种类。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<MediaKind>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn media_source(extra: &str) -> String {
        format!(
            r#"{{"service": "sidecar", "port": 8090, "credentials": {{"issue": ["i", "{{name}}"], "revoke": ["r", "{{name}}"]}}{extra}}}"#
        )
    }

    #[test]
    fn media_lists_roundtrip_and_default_to_empty() {
        let json = media_source(r#", "operations": ["upload", "list"], "kinds": ["photo"]"#);
        let provider: ProviderService = serde_json::from_str(&json).unwrap();
        assert_eq!(
            provider.operations,
            vec![MediaOperation::Upload, MediaOperation::List]
        );
        assert_eq!(provider.kinds, vec![MediaKind::Photo]);
        let back = serde_json::to_value(&provider).unwrap();
        assert_eq!(back["operations"], serde_json::json!(["upload", "list"]));
        assert_eq!(back["kinds"], serde_json::json!(["photo"]));

        let provider: ProviderService = serde_json::from_str(&media_source("")).unwrap();
        assert!(provider.operations.is_empty());
        assert!(provider.kinds.is_empty());
        let back = serde_json::to_value(&provider).unwrap();
        assert!(back.get("operations").is_none());
        assert!(back.get("kinds").is_none());
    }

    #[test]
    fn unknown_media_list_values_fall_to_other() {
        let json = media_source(
            r#", "operations": ["upload", "transcode"], "kinds": ["photo", "hologram"]"#,
        );
        let provider: ProviderService = serde_json::from_str(&json).unwrap();
        assert_eq!(
            provider.operations,
            vec![
                MediaOperation::Upload,
                MediaOperation::Other("transcode".into())
            ]
        );
        assert_eq!(
            provider.kinds,
            vec![MediaKind::Photo, MediaKind::Other("hologram".into())]
        );
        let back = serde_json::to_value(&provider).unwrap();
        assert_eq!(
            back["operations"],
            serde_json::json!(["upload", "transcode"])
        );
        assert_eq!(back["kinds"], serde_json::json!(["photo", "hologram"]));
    }

    #[test]
    fn credentials_need_both_commands() {
        let json =
            r#"{"service": "sidecar", "port": 8090, "credentials": {"issue": ["i", "{name}"]}}"#;
        assert!(serde_json::from_str::<ProviderService>(json).is_err());
        let json =
            r#"{"service": "sidecar", "port": 8090, "credentials": {"revoke": ["r", "{name}"]}}"#;
        assert!(serde_json::from_str::<ProviderService>(json).is_err());
    }
}
