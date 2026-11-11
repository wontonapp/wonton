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
    #[serde(default)]
    pub skills: Option<AgentSkills>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSkills {
    pub mount: String,
}

pub(super) fn validate(manifest: &Manifest) -> Result<(), ManifestError> {
    if manifest.kind == PluginKind::Agent && manifest.agent.is_none() {
        return Err(ManifestError::Invalid(
            "kind is agent, so agent.protocol is required".to_owned(),
        ));
    }
    let Some(agent) = &manifest.agent else {
        return Ok(());
    };
    if let Some(skills) = &agent.skills {
        if manifest.kind != PluginKind::Agent {
            return Err(ManifestError::Invalid(
                "agent.skills.mount requires kind: agent".to_owned(),
            ));
        }
        validate_mount("agent.skills.mount", &skills.mount)?;
    }
    let mut seen = std::collections::BTreeSet::new();
    for login in &agent.login {
        if !is_label(&login.id) {
            return Err(ManifestError::Invalid(format!(
                "agent.login id `{}` is invalid",
                login.id
            )));
        }
        if !seen.insert(&login.id) {
            return Err(ManifestError::Invalid(format!(
                "agent.login id `{}` is duplicated",
                login.id
            )));
        }
        require_locales("agent.login.title", &login.title)?;
    }
    if !agent.login.is_empty() && !manifest.permissions.contains(&Permission::NetworkEgress) {
        return Err(ManifestError::Invalid(
            "agent.login requires the network.egress permission".to_owned(),
        ));
    }
    Ok(())
}

/// 容器里的挂载点：绝对路径，不是 `/`，不带 `..`，不在 `/proc`、`/sys`、`/dev` 下。
pub(super) fn validate_mount(field: &str, mount: &str) -> Result<(), ManifestError> {
    let first = mount
        .split('/')
        .find(|part| !part.is_empty() && *part != ".");
    if !mount.starts_with('/')
        || mount.contains('\0')
        || mount.split('/').any(|part| part == "..")
        || first.is_none()
        || matches!(first, Some("proc" | "sys" | "dev"))
    {
        return Err(ManifestError::Invalid(format!(
            "{field} `{mount}` must be an absolute container path other than /, \
             without .. and outside /proc, /sys and /dev"
        )));
    }
    Ok(())
}
