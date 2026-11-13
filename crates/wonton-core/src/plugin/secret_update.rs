//! 改插件的一项填写密钥（ADR-0064）：改完只重建渲染结果里用到这个变量的服务，值不返回。

use super::health_gate::Gate;
use super::install_error::InstallError;
use super::languages::Languages;
use super::listed::Listed;
use super::manager::{PluginManager, detached, now_secs};
use super::manifest::{Manifest, PluginId};
use super::package::PluginPackage;
use super::render::render;
use super::secrets::Secrets;
use super::store::InstalledRecord;
use super::{secret_input, secrets};
use std::sync::Arc;
use wonton_protocol::PluginDetail;

impl PluginManager {
    /// 填一项或清空可选项。`value` 为空是清空。成功返回更新后的详情；用到这个变量的服务会被重建。
    /// 在独立的任务里执行：请求断开也照样做完。
    pub(crate) async fn set_secret_input(
        self: &Arc<Self>,
        id: &PluginId,
        name: &str,
        value: Option<&str>,
        langs: &Languages,
    ) -> Result<PluginDetail, InstallError> {
        let (this, id, name) = (self.clone(), id.clone(), name.to_owned());
        let (value, langs) = (value.map(str::to_owned), langs.clone());
        detached(async move {
            this.set_secret_input_inner(&id, &name, value.as_deref(), &langs)
                .await
        })
        .await
    }

    async fn set_secret_input_inner(
        &self,
        id: &PluginId,
        name: &str,
        value: Option<&str>,
        langs: &Languages,
    ) -> Result<PluginDetail, InstallError> {
        let _pass = self.lan.pass.lock().await;
        let listed = self.listed(id)?;
        let record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::NotInstalled(id.to_string()))?;
        let package = self.package_of(&record)?;
        let before = self.store.secrets().load(id)?.unwrap_or_default();
        let before_changed = self.store.secrets().changed(id)?;
        let mut stored = before.clone();
        let mut changed = before_changed.clone();
        if !secret_input::store(&package.manifest, &mut stored, name, value, langs)? {
            return self
                .judged_detail(id, langs)
                .await
                .ok_or_else(|| not_installed(id));
        }
        if !self.begin_rebuild(id) {
            return Err(InstallError::Conflict(format!(
                "插件 {id} 正在安装、卸载或重建，等那边做完"
            )));
        }
        changed.insert(name.to_owned(), now_secs() * 1000);
        if let Err(e) = self.store.secrets().save_changed(id, &stored, &changed) {
            self.lan.end(id);
            return Err(e.into());
        }
        let result = self
            .rebuild_secret_users(&listed, &package, &record, name, langs)
            .await;
        if let Err(e) = &result {
            // 值退回原样再照旧渲染一次：重建可能已经删了旧容器或起了一半新的。
            let _ = self
                .store
                .secrets()
                .save_changed(id, &before, &before_changed);
            self.refresh_mcp_headers(id, &package.manifest, &before, langs);
            match self
                .rebuild_secret_users(&listed, &package, &record, name, langs)
                .await
            {
                Ok(()) => self.changed(),
                Err(back) => {
                    tracing::error!(plugin = %id, secret = %name, error = %back, "改密钥失败，退回旧值也失败");
                    self.set_degraded(id, format!("改密钥失败，退回也失败：{back}"));
                }
            }
            tracing::warn!(plugin = %id, secret = %name, error = %e, "改密钥失败");
        } else {
            self.refresh_mcp_headers(id, &package.manifest, &stored, langs);
            tracing::info!(plugin = %id, secret = %name, "主人改了一项密钥");
            self.changed();
        }
        self.lan.end(id);
        result?;
        self.judged_detail(id, langs)
            .await
            .ok_or_else(|| not_installed(id))
    }

    /// `mcp.headers` 里引用的密钥换了值：按新值重新解析到插件表的连接上。填齐必填项后，MCP 的未就绪和这
    /// 时空了头的请求头一起恢复。解析不出来（别的密钥有问题）就留着旧头，等下次核对。
    fn refresh_mcp_headers(
        &self,
        id: &PluginId,
        manifest: &Manifest,
        secrets: &Secrets,
        langs: &Languages,
    ) {
        let Ok(headers) =
            super::proxy_auth::resolve_mcp_headers_or_pending(manifest, secrets, langs)
        else {
            return;
        };
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(target) = plugins.get_mut(id).and_then(|entry| entry.mcp.as_mut()) {
            target.headers = headers;
        }
    }

    /// 只重建 env 里有 `WONTON_SECRET_<名字>` 的服务；没有服务用它就只存值，下次启动生效。
    async fn rebuild_secret_users(
        &self,
        listed: &Listed,
        package: &PluginPackage,
        record: &InstalledRecord,
        name: &str,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let variable = secrets::secret_var(name);
        let plan = self.addresses_for(listed, record.lan_webui, langs).await?;
        let addresses: Vec<_> = plan.bind.iter().map(|bound| bound.ip).collect();
        let grant = (!addresses.is_empty())
            .then(|| self.lan_grant_on(package, record.lan_webui, addresses, plan.announce));
        let mut ctx = self.rebuild_context(package, record, grant, None).await?;
        self.fill_bot_vars(package, &mut ctx, langs).await?;
        let mut rendered = render(package, &ctx, langs)?;
        let users: std::collections::BTreeSet<String> = rendered
            .containers
            .iter()
            .filter(|container| container.variables.contains(&variable))
            .map(|container| container.service.clone())
            .collect();
        if users.is_empty() {
            tracing::info!(plugin = %record.id, secret = %name, "没有服务用到这项密钥，下次启动时注入");
            return Ok(());
        }
        rendered.retain_services(&users);
        self.replace_containers(&ctx, rendered, Gate::StartPeriod)
            .await?;
        self.recovery_complete(&record.id)
    }
}

fn not_installed(id: &PluginId) -> InstallError {
    InstallError::NotInstalled(id.to_string())
}
