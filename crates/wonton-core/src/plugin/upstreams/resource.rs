//! 核心访问插件资源（共享文件夹）的内网地址，以及运行状态和授权查询。

use super::super::manager::{PluginManager, PluginStatus};
use super::super::manifest::{Permission, PluginId};
use super::super::names;
use super::super::package::PluginPackage;
use super::super::store::StoreError;

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
    pub(crate) fn is_running(&self, id: &PluginId) -> bool {
        self.status_of(id) == Some(PluginStatus::Running)
    }

    /// 这个插件现在有没有这个共享文件夹的授权，读或写都算（ADR-0020 的“鉴权”）。卸了的插件没有。
    pub fn share_granted(&self, id: &PluginId, share: &str) -> Result<bool, StoreError> {
        let Some(record) = self.store.load(id)? else {
            return Ok(false);
        };
        Ok(record
            .granted_permissions
            .iter()
            .any(|p| matches!(p, Permission::StorageShare { name, .. } if name == share)))
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
        let package =
            PluginPackage::with_files(&record.source, record.files, &self.owner_languages())
                .map_err(|e| internal(e.to_string()))?;
        if !package.has_service(service) {
            return Err(ResourceUpstreamError::NotItsService);
        }
        let slug = id.slug();
        let name = self.cfg.instance.container(&slug, service);
        let network = self.cfg.instance.network(&slug);
        self.service_address(&name, &names::service_host(&slug, service), port, &network)
            .await
            .map_err(|e| internal(e.to_string()))?
            .ok_or(ResourceUpstreamError::Unavailable)
    }
}
