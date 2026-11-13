//! 客户端 HTTP 入口的部署地址；初始化检查见 [`super::clients_setup`]。

use super::manager::{ClientsRoute, InstallError, PluginManager};
use super::manifest::Manifest;
use super::names;
use super::render::Deployment;
use super::upstreams::Endpoint;

/// `setup_done` 的一次结果（ADR-0022）：退出码 0 是完成，1 是还没有；其他退出码和超时算查不出，
/// 标准错误输出交给入口回 503 `plugin_unavailable`。
#[derive(Clone)]
pub(crate) enum SetupCheck {
    Done,
    Pending,
    Undetermined(String),
    Unavailable,
}

impl PluginManager {
    /// 声明了 `clients.http` 时的转发地址：核心在容器里时按服务的稳定域名（`服务名.<slug>.wonton.internal`）
    /// 解析出地址。网页的宿主网络中继不适用：客户端入口只走插件网络（ADR-0021）。
    pub(super) async fn clients_upstream(
        &self,
        manifest: &Manifest,
        plan: &Deployment,
    ) -> Result<Option<ClientsRoute>, InstallError> {
        let Some(http) = manifest
            .clients
            .as_ref()
            .and_then(|clients| clients.http.as_ref())
        else {
            return Ok(None);
        };
        let slug = manifest.id.slug();
        let service = plan
            .clients_http_network_service(&http.service)
            .ok_or_else(|| {
                InstallError::Invalid(crate::l10n::text(
                    &self.owner_languages(),
                    "gateway-entry-target-unavailable",
                    &[],
                ))
            })?;
        let name = self.cfg.instance.container(&slug, service);
        let address = self
            .service_address(
                &name,
                &names::service_host(&slug, service),
                http.port,
                &plan.network,
            )
            .await?;
        Ok(Some(ClientsRoute {
            http: http.clone(),
            upstream: address.map(Endpoint::Tcp),
        }))
    }
}
