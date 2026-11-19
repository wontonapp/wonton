//! 插件清单里的 Agent 协议声明与校验。

use super::manifest::{Manifest, ManifestError, PluginKind};

pub use wonton_protocol::manifest::{AgentProtocol, AgentSpec};

pub(super) fn validate(manifest: &Manifest) -> Result<(), ManifestError> {
    if manifest.kind == PluginKind::Agent && manifest.agent.is_none() {
        return Err(ManifestError::Invalid(
            "kind is agent, so agent.protocol is required".to_owned(),
        ));
    }
    let Some(agent) = &manifest.agent else {
        return Ok(());
    };
    if agent.protocol == AgentProtocol::Acp {
        return Err(ManifestError::UnsupportedAgentProtocol {
            protocol: "acp".to_owned(),
        });
    }
    if let Some(skills) = &agent.skills {
        if manifest.kind != PluginKind::Agent {
            return Err(ManifestError::Invalid(
                "agent.skills.mount requires kind: agent".to_owned(),
            ));
        }
        validate_mount("agent.skills.mount", &skills.mount)?;
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
