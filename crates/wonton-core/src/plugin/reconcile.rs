//! 核心重启或巡检时核对已装的插件：容器、网络、OIDC 客户端和密钥是否还和记录一致。

use std::sync::Arc;

use super::docker::NetworkReach;
use super::health::{self, Checked};
use super::lifecycle::client_spec;
use super::manager::{Deployed, Health, InstallError, PluginManager};
use super::manifest::Manifest;
use super::package::PluginPackage;
use super::proxy_auth::ProxyAuth;
use super::secrets::{self, Secrets};
use super::store::InstalledRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ContainersState {
    Running(Health),
    Stopped,
}

impl PluginManager {
    /// 核心重启后核对插件：一次性服务已以 0 退出，长期运行的容器在运行、健康检查没有失败；核心在插件
    /// 网络里、OIDC 客户端已登记、插件密钥都在，并重新收存一次插件写下的密钥。`None` 表示常驻服务全部停止。
    pub(super) async fn reconcile(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
    ) -> Result<Option<Deployed>, InstallError> {
        let manifest = &package.manifest;
        let slug = manifest.id.slug();
        let network = self.cfg.instance.network(&slug);
        let expected = self.network_reach(manifest);
        let actual = self.engine.network_reach(&network).await?;
        if actual != Some(expected) {
            return Err(InstallError::Conflict(format!(
                "插件网络 {network} {}，按清单和核心的运行方式应该{}：重装插件（可以不删数据）后生效",
                describe_reach(actual),
                describe_reach(Some(expected))
            )));
        }
        // 先接核心再看容器：靠核心的内部别名才起得来的插件，核心不接回去就一直崩，先看容器的话两边互相等。
        if let Some(core) = &self.cfg.core_container {
            self.attach_core(&network, core, manifest).await?;
        }
        let health = match self.check_containers(package).await? {
            ContainersState::Running(health) => health,
            ContainersState::Stopped => return Ok(None),
        };
        self.ensure_relays(manifest, record).await?;
        self.check_bot(manifest).await?;
        let secrets = self.check_secrets(manifest).await?;
        let generated =
            secrets::generated_values(manifest, &secrets).map_err(InstallError::Storage)?;
        let proxy = ProxyAuth::resolve(manifest, &generated).map_err(InstallError::Storage)?;
        if let Some(secret) = &record.oidc_client_secret {
            let ctx = self
                .install_context(manifest, Some(secret), generated)
                .await?;
            self.hydra()?
                .upsert_client(&client_spec(manifest, &ctx, secret))
                .await?;
        }
        Ok(Some(Deployed {
            upstream: self.webui_upstream(manifest, &network).await?,
            health,
            models: self.models_upstream(manifest, &network).await?,
            mcp: self.mcp_upstream(manifest, &network, &secrets).await?,
            proxy: proxy.map(Arc::new),
            firewalled: record.firewalled,
        }))
    }

    pub(super) async fn reconcile_running(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
    ) -> Result<Deployed, InstallError> {
        match self.reconcile(package, record).await? {
            Some(deployed) => Ok(deployed),
            None => Err(InstallError::Unhealthy("插件的常驻服务已停止".to_owned())),
        }
    }

    /// 看一眼插件的每个容器，不等待。不在应有的状态是 [`InstallError::Unhealthy`]。
    pub(super) async fn check_containers(
        &self,
        package: &PluginPackage,
    ) -> Result<ContainersState, InstallError> {
        let slug = package.manifest.id.slug();
        let one_shot = package.one_shot_services();
        let mut running = 0;
        let mut healthy = true;
        let mut stopped = None;
        for service in package.service_names() {
            let name = self.cfg.instance.container(&slug, service);
            match health::check_now(&self.engine, &name, service, one_shot.contains(service))
                .await?
            {
                Checked::Completed => {}
                Checked::Healthy => running += 1,
                Checked::Running => {
                    running += 1;
                    healthy = false;
                }
                Checked::Stopped => stopped = Some(name),
                Checked::Failed(reason) => return Err(InstallError::Unhealthy(reason)),
            }
        }
        if let Some(container) = stopped {
            return if running == 0 {
                Ok(ContainersState::Stopped)
            } else {
                Err(InstallError::Unhealthy(format!(
                    "部分常驻服务仍在运行，容器 {container} 已停止"
                )))
            };
        }
        Ok(ContainersState::Running(if healthy {
            Health::Healthy
        } else {
            Health::Unknown
        }))
    }

    /// 核对插件密钥：生成的都还在，插件写下的重新收存一次，变了就更新。返回全量密钥（generate 和
    /// collect）：generate 项的文本值经 [`secrets::generated_values`] 取，MCP 头要引用 collect 项。
    pub(super) async fn check_secrets(&self, manifest: &Manifest) -> Result<Secrets, InstallError> {
        if manifest.secrets.is_empty() {
            return Ok(Secrets::new());
        }
        let store = self.store.secrets();
        let mut secrets = store.load(&manifest.id)?.unwrap_or_default();
        secrets::generated_values(manifest, &secrets).map_err(InstallError::Storage)?;
        if secrets::collect(&self.engine, &self.cfg.instance, manifest, &mut secrets).await? {
            store.save(&manifest.id, &secrets)?;
        }
        Ok(secrets)
    }
}

fn describe_reach(reach: Option<NetworkReach>) -> &'static str {
    match reach {
        None => "不存在",
        Some(NetworkReach::Egress) => "能出网",
        Some(NetworkReach::Internal) => "不能出网",
        Some(NetworkReach::Isolated) => "不能出网、和宿主隔开",
    }
}
