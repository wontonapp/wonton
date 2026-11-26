//! Docker 守护进程和宿主信息：版本、架构、运行时，以及最低引擎版本检查。

use super::super::languages::Languages;
use super::{Engine, EngineError, HostInfo, MIN_ENGINE_MAJOR, Result, ServerInfo};
use crate::l10n;

impl Engine {
    /// 检查 Docker 可达，返回宿主架构（如 `x86_64`、`aarch64`）。
    pub async fn host_arch(&self) -> Result<String> {
        Ok(self.server().await?.arch)
    }

    async fn server(&self) -> Result<ServerInfo> {
        let version = self
            .docker
            .version()
            .await
            .map_err(|e| self.fail("plugin-install-docker-context-version", &[], e))?;
        Ok(ServerInfo {
            version: version.version.unwrap_or_default(),
            arch: version.arch.unwrap_or_default(),
        })
    }

    /// Docker 登记的 OCI 运行时的名字（`runc`，装了 NVIDIA Container Toolkit 时还有 `nvidia`）。
    pub async fn runtimes(&self) -> Result<Vec<String>> {
        Ok(self.host_info().await?.runtimes)
    }

    pub async fn host_info(&self) -> Result<HostInfo> {
        let info = self
            .docker
            .info()
            .await
            .map_err(|e| self.fail("plugin-install-docker-context-host-info", &[], e))?;
        let mut runtimes: Vec<String> = info.runtimes.unwrap_or_default().into_keys().collect();
        runtimes.sort();
        Ok(HostInfo {
            name: info.name,
            os_type: info.os_type.unwrap_or_default(),
            arch: info.architecture.unwrap_or_default(),
            runtimes,
            memory_limit: info.memory_limit,
            swap_limit: info.swap_limit,
            cpu_shares: info.cpu_shares,
            pids_limit: info.pids_limit,
            ncpu: info.ncpu,
            cgroup_driver: info.cgroup_driver.map(|value| value.to_string()),
            cgroup_version: info.cgroup_version.map(|value| value.to_string()),
        })
    }
    /// 核心启动时检查 Docker：连得上，而且 Engine 版本不低于 [`MIN_ENGINE_MAJOR`]。低于时报错，不降级运行：
    /// 插件网络的隔离靠的是新版的行为。
    pub async fn check_server(&self) -> Result<ServerInfo> {
        let info = self.server().await?;
        check_engine_version(&self.owner_languages(), &info.version)?;
        Ok(info)
    }
}

pub(super) fn check_engine_version(langs: &Languages, version: &str) -> Result<()> {
    let major = version
        .split('.')
        .next()
        .and_then(|m| m.parse::<u32>().ok());
    match major {
        Some(major) if major >= MIN_ENGINE_MAJOR => Ok(()),
        Some(_) => Err(EngineError::Unsupported(l10n::text(
            langs,
            "plugin-install-docker-engine-too-old",
            &[
                ("version", version.into()),
                ("min", MIN_ENGINE_MAJOR.into()),
            ],
        ))),
        None => Err(EngineError::Unsupported(l10n::text(
            langs,
            "plugin-install-docker-engine-version-unrecognized",
            &[("version", version.into())],
        ))),
    }
}
