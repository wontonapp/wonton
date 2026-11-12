//! 通用能力按容器实际网络模式选择地址，与安装记录使用同一中继标识。

use super::super::host_relay::Purpose;
use super::super::names;
use super::{Endpoint, InstallError, Manifest, PluginManager};

impl PluginManager {
    pub(in crate::plugin) async fn capability_endpoint(
        &self,
        manifest: &Manifest,
        service: &str,
        port: u16,
    ) -> Result<Option<Endpoint>, InstallError> {
        if let Some(endpoint) = self
            .host_relay_endpoint(manifest, Purpose::Capability, service, port)
            .await?
        {
            return Ok(Some(endpoint));
        }
        let slug = manifest.id.slug();
        self.service_address(
            &self.cfg.instance.container(&slug, service),
            &names::service_host(&slug, service),
            port,
            &self.cfg.instance.network(&slug),
        )
        .await
        .map(|address| address.map(Endpoint::Tcp))
    }
}
