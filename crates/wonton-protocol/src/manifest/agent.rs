use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentProtocol {
    Matrix,
    Acp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    pub protocol: AgentProtocol,
    #[serde(default)]
    pub skills: Option<AgentSkills>,
    /// 模型接入命令（ADR-0129）：App 经核心调它列服务商、填 key、登录、选模型和测试。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<AgentModels>,
}

/// `agent.models`：核心经 Docker exec 在 `service` 里执行 `command`（argv，不经 shell），一次一个操作，
/// 输入输出见 [`crate::ModelsCommandInput`]。有 `bots` 的插件在这个 Bot 所在那一组的服务里执行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentModels {
    pub service: String,
    pub command: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSkills {
    pub mount: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_command_is_optional_and_strict() {
        let plain: AgentSpec = serde_json::from_str(r#"{"protocol": "matrix"}"#).unwrap();
        assert_eq!(plain.models, None);
        let spec: AgentSpec = serde_json::from_str(
            r#"{"protocol": "matrix", "models": {"service": "gateway", "command": ["python", "/m.py"]}}"#,
        )
        .unwrap();
        assert_eq!(
            spec.models,
            Some(AgentModels {
                service: "gateway".into(),
                command: vec!["python".into(), "/m.py".into()],
            })
        );
        let typo = r#"{"protocol": "matrix", "models": {"service": "gateway", "command": [], "cmd": []}}"#;
        assert!(serde_json::from_str::<AgentSpec>(typo).is_err());
    }
}
