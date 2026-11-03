//! 核心和插件之间的入口：核心在插件网络里的别名（插件经它们访问核心转发的服务），以及核心访问插件的地址
//! （网页、模型网关、MCP 服务）。

use super::PluginId;
use super::manager::{InstallError, PluginManager, PluginStatus};
use super::manifest::{Manifest, Permission, Sso};
use super::models::ModelsRoute;
use super::package::PluginPackage;
use crate::config::{PLUGIN_MATRIX_HOST, PLUGIN_MCP_HOST, PLUGIN_MODELS_HOST, PublicUrls};

/// MCP 服务没写 `path` 时的路径。
const DEFAULT_MCP_PATH: &str = "/mcp";

pub(super) fn uses_sso(manifest: &Manifest) -> bool {
    manifest.webui.as_ref().is_some_and(|w| w.sso == Sso::Oidc)
}

/// 核心在插件网络里的别名：插件经它们访问核心代为转发的服务。SSO 插件经 `auth.<base>` 访问身份提供方，
/// Bot 经 `matrix.wonton.internal` 访问 Matrix 客户端接口，模型网关的使用方经 `models.wonton.internal`，
/// Agent 经 `mcp.wonton.internal` 访问 MCP 总线。
pub(super) fn core_aliases(public: &PublicUrls, manifest: &Manifest) -> Vec<String> {
    let mut aliases = Vec::new();
    if uses_sso(manifest) {
        aliases.push(public.auth_host());
    }
    let granted = |permission: Permission| manifest.permissions.contains(&permission);
    if granted(Permission::MatrixBot) {
        aliases.push(PLUGIN_MATRIX_HOST.to_owned());
    }
    if granted(Permission::Models) {
        aliases.push(PLUGIN_MODELS_HOST.to_owned());
    }
    if granted(Permission::Tools) {
        aliases.push(PLUGIN_MCP_HOST.to_owned());
    }
    aliases
}

/// [`PluginManager::resource_upstream`] 给不出地址的原因。
#[derive(Debug, thiserror::Error)]
pub enum ResourceUpstreamError {
    #[error("插件卸了或没在运行")]
    Unavailable,
    #[error("主机名不是这个插件 compose 里的服务名")]
    NotItsService,
    /// 读不了插件记录、解析不了插件包、Docker 出错：核心这边的问题，和插件是否在运行无关。
    #[error("{0}")]
    Internal(String),
}

impl PluginManager {
    /// 核心访问插件网页的地址。
    pub(super) async fn webui_upstream(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<Option<String>, InstallError> {
        let Some(webui) = &manifest.webui else {
            return Ok(None);
        };
        let name = self
            .cfg
            .instance
            .container(&manifest.id.slug(), &webui.service);
        self.service_upstream(&name, webui.port, network)
            .await
            .map(Some)
    }

    /// 是模型网关时：核心转发模型请求、执行凭据命令的目标。
    pub(super) async fn models_upstream(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<Option<ModelsRoute>, InstallError> {
        let Some(spec) = manifest.provides.as_ref().and_then(|p| p.models.as_ref()) else {
            return Ok(None);
        };
        let slug = manifest.id.slug();
        let name = self.cfg.instance.container(&slug, &spec.service);
        let upstream = self.service_upstream(&name, spec.port, network).await?;
        Ok(Some(ModelsRoute {
            id: manifest.id.clone(),
            spec: spec.clone(),
            upstream,
            credentials_container: self
                .cfg
                .instance
                .container(&slug, spec.credentials_service()),
        }))
    }

    /// 声明了 `mcp` 时：MCP 总线连插件 MCP 服务的地址（`http://主机:端口/路径`）。
    pub(super) async fn mcp_upstream(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<Option<String>, InstallError> {
        let Some(mcp) = &manifest.mcp else {
            return Ok(None);
        };
        let name = self
            .cfg
            .instance
            .container(&manifest.id.slug(), &mcp.service);
        let upstream = self.service_upstream(&name, mcp.port, network).await?;
        let path = mcp.path.as_deref().unwrap_or(DEFAULT_MCP_PATH);
        Ok(Some(format!("http://{upstream}{path}")))
    }

    pub(crate) fn is_running(&self, id: &PluginId) -> bool {
        self.status_of(id) == Some(PluginStatus::Running)
    }

    /// 核心访问插件资源的内网地址（ADR-0020）。主机名必须是这个插件 compose 里的服务名；不同插件可能有同名的服务，所以换成这个
    /// 插件的容器的地址，不按主机名解析。
    pub async fn resource_upstream(
        &self,
        id: &PluginId,
        service: &str,
        port: u16,
    ) -> Result<String, ResourceUpstreamError> {
        if !self.is_running(id) {
            return Err(ResourceUpstreamError::Unavailable);
        }
        let internal = |why: String| ResourceUpstreamError::Internal(why);
        let record = self
            .store
            .load(id)
            .map_err(|e| internal(e.to_string()))?
            .ok_or(ResourceUpstreamError::Unavailable)?;
        let package = PluginPackage::with_files(&record.source, record.files)
            .map_err(|e| internal(e.to_string()))?;
        if !package.has_service(service) {
            return Err(ResourceUpstreamError::NotItsService);
        }
        let slug = id.slug();
        let name = self.cfg.instance.container(&slug, service);
        let network = self.cfg.instance.network(&slug);
        self.service_address(&name, port, &network)
            .await
            .map_err(|e| internal(e.to_string()))?
            .ok_or(ResourceUpstreamError::Unavailable)
    }

    /// 核心访问容器里一个端口的地址：核心在容器里时用容器名（同一网络内的 DNS），否则用容器 IP。
    pub(super) async fn service_upstream(
        &self,
        name: &str,
        port: u16,
        network: &str,
    ) -> Result<String, InstallError> {
        self.service_address(name, port, network)
            .await?
            .ok_or_else(|| {
                InstallError::Docker(format!(
                    "容器 {name} 不存在，或者在网络 {network} 中没有地址"
                ))
            })
    }

    /// 容器不在或者在这个网络里没有地址时为空。
    async fn service_address(
        &self,
        name: &str,
        port: u16,
        network: &str,
    ) -> Result<Option<String>, InstallError> {
        if self.cfg.core_container.is_some() {
            return Ok(Some(format!("{name}:{port}")));
        }
        let info = self.engine.inspect_container(name).await?;
        let ip = info
            .and_then(|info| info.network_settings)
            .and_then(|s| s.networks)
            .and_then(|mut n| n.remove(network))
            .and_then(|e| e.ip_address)
            .filter(|ip| !ip.is_empty());
        Ok(ip.map(|ip| format!("{ip}:{port}")))
    }
}
