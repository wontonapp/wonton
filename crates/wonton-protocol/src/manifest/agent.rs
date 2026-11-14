use serde::{Deserialize, Serialize};

use super::LocalizedText;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentProtocol {
    Matrix,
    Acp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentLogin {
    pub id: String,
    pub title: LocalizedText,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    pub protocol: AgentProtocol,
    #[serde(default)]
    pub login: Vec<AgentLogin>,
    #[serde(default)]
    pub skills: Option<AgentSkills>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSkills {
    pub mount: String,
}
