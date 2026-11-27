//! 插件清单里的 Agent 协议声明与校验。

use super::manifest::{Manifest, ManifestError, PluginKind};

pub use wonton_protocol::manifest::{AgentModels, AgentProtocol, AgentSpec};

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
    if let Some(models) = &agent.models {
        validate_models(manifest, models)?;
    }
    Ok(())
}

/// `agent.models`：只给 Agent 插件；写了 `bots` 的插件，命令要在每个 Bot 自己的那组服务里跑，否则一个 Bot 的 key
/// 会存进所有 Bot 共用的容器。
fn validate_models(manifest: &Manifest, models: &AgentModels) -> Result<(), ManifestError> {
    if manifest.kind != PluginKind::Agent {
        return Err(ManifestError::Invalid(
            "agent.models requires kind: agent".to_owned(),
        ));
    }
    // 服务名原样拿去找容器：前后带空格的名字装得上，运行时却永远找不到。
    if models.service.trim().is_empty()
        || models.service.trim() != models.service
        || models
            .command
            .first()
            .is_none_or(|program| program.trim().is_empty())
    {
        return Err(ManifestError::Invalid(
            "agent.models needs a service name without spaces around it and a non-empty command"
                .to_owned(),
        ));
    }
    if manifest.bots.is_some()
        && !super::bot_services::bot_services(manifest).contains(models.service.as_str())
    {
        return Err(ManifestError::Invalid(format!(
            "agent.models.service `{}` must be listed in bots.services: the command runs for one Bot at a time",
            models.service
        )));
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
