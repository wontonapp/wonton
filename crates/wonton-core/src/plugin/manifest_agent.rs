//! 插件清单里的 Agent 协议、账号登录声明与校验。

use serde::{Deserialize, Serialize};

use super::manifest::{
    LocalizedText, Manifest, ManifestError, Permission, PluginKind, is_label, require_locales,
};

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
}

pub(super) fn validate(manifest: &Manifest) -> Result<(), ManifestError> {
    if manifest.kind == PluginKind::Agent && manifest.agent.is_none() {
        return Err(ManifestError::Invalid(
            "kind 为 agent 时必须提供 agent.protocol".to_owned(),
        ));
    }
    let Some(agent) = &manifest.agent else {
        return Ok(());
    };
    let mut seen = std::collections::BTreeSet::new();
    for login in &agent.login {
        if !is_label(&login.id) {
            return Err(ManifestError::Invalid(format!(
                "agent.login id `{}` 不合法",
                login.id
            )));
        }
        if !seen.insert(&login.id) {
            return Err(ManifestError::Invalid(format!(
                "agent.login id `{}` 重复了",
                login.id
            )));
        }
        require_locales("agent.login.title", &login.title)?;
    }
    if !agent.login.is_empty() && !manifest.permissions.contains(&Permission::NetworkEgress) {
        return Err(ManifestError::Invalid(
            "声明了 agent.login 就必须申请 network.egress 权限".to_owned(),
        ));
    }
    Ok(())
}
