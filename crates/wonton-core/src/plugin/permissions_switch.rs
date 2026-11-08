//! 开关一项可选权限（ADR-0057）：开着的拿到对应字段，关着的拿掉，其余服务不动，数据不动。
//!
//! 和 `lan_webui` 的开关共用 `begin_rebuild` 和 `replace_containers`：两边都要重建容器，串行执行。渲染前后两份计划，
//! 只重启渲染结果真的变了的服务（字段变了，或者 `WONTON_PERMISSION_*` 落进了它的环境变量）；失败时记录和容器都回到
//! 原来的样子。

use std::collections::BTreeSet;
use std::sync::Arc;

use wonton_protocol::PluginDetail;

use super::entry::PluginStatus;
use super::languages::Languages;
use super::listed::Listed;
use super::manager::{InstallError, PluginManager, detached};
use super::manifest::{Permission, PluginId};
use super::package::PluginPackage;
use super::render::{ContainerPlan, Deployment, InstallContext, render};
use super::store::{EnabledPermission, InstalledRecord, OpenedBy};

impl PluginManager {
    /// 改一项可选权限，重建完返回详情。在独立的任务里执行，请求断开了也照样做完。
    pub async fn set_plugin_permission(
        self: &Arc<Self>,
        id: PluginId,
        permission: Permission,
        enabled: bool,
        opened_by: OpenedBy,
        langs: Languages,
    ) -> Result<PluginDetail, InstallError> {
        let this = Arc::clone(self);
        detached(async move {
            this.switch_permission(&id, &permission, enabled, opened_by)
                .await?;
            this.judged_detail(&id, &langs)
                .await
                .ok_or_else(|| InstallError::NotInstalled(id.to_string()))
        })
        .await
    }

    async fn switch_permission(
        &self,
        id: &PluginId,
        permission: &Permission,
        enabled: bool,
        opened_by: OpenedBy,
    ) -> Result<(), InstallError> {
        let _pass = self.lan.pass.lock().await;
        let listed = self.listed(id)?;
        if listed.manifest.optional_permission(permission).is_none() {
            return Err(InstallError::Invalid(format!(
                "插件 {id} 没有把 {permission} 声明成可选，不能开关"
            )));
        }
        let mut record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::NotInstalled(id.to_string()))?;
        let already = record
            .enabled_permissions
            .iter()
            .any(|item| &item.permission == permission);
        if already == enabled {
            return Ok(());
        }
        let before = record.clone();
        if enabled {
            record.enabled_permissions.push(EnabledPermission {
                permission: permission.clone(),
                since: super::manager::now_secs(),
                opened_by,
            });
        } else {
            record
                .enabled_permissions
                .retain(|item| &item.permission != permission);
        }
        if !self.begin_rebuild(id) {
            return Err(InstallError::Conflict(format!(
                "插件 {id} 正在安装、卸载或重建，等那边做完"
            )));
        }
        let package = self.package_of(&before)?;
        let result = self
            .apply_permission(&listed, &package, &before, &record)
            .await;
        if let Err(e) = &result {
            // 回到原来的样子：照旧记录渲染一次。
            if let Err(back) = self
                .apply_permission(&listed, &package, &record, &before)
                .await
            {
                tracing::error!(plugin = %id, error = %back, "改可选权限失败，退回旧状态也失败");
                self.set_degraded(id, format!("改可选权限失败，退回也失败：{back}"));
            }
            tracing::warn!(plugin = %id, permission = %permission, enabled, error = %e, "改可选权限失败");
        } else {
            tracing::info!(plugin = %id, permission = %permission, enabled, "主人改了可选权限");
            self.changed();
        }
        self.lan.end(id);
        result
    }

    /// 按 `from`、`to` 的生效集各渲染一次，替换渲染结果变了的容器，成功后把 `to` 存成新记录。
    async fn apply_permission(
        &self,
        listed: &Listed,
        package: &PluginPackage,
        from: &InstalledRecord,
        to: &InstalledRecord,
    ) -> Result<(), InstallError> {
        let langs = self.owner_languages();
        let before = self.render_for(listed, package, from, &langs).await?;
        let after = self.render_for(listed, package, to, &langs).await?;
        let changed = changed_services(&before, &after);
        if changed.is_empty() {
            self.store.save(to)?;
            return Ok(());
        }
        let mut plan = after;
        plan.retain_services(&changed);
        let ctx = self.switch_context(listed, package, to, &langs).await?;
        self.replace_containers(&ctx, plan).await?;
        self.store.save(to)?;
        Ok(())
    }

    async fn render_for(
        &self,
        listed: &Listed,
        package: &PluginPackage,
        record: &InstalledRecord,
        langs: &Languages,
    ) -> Result<Deployment, InstallError> {
        let ctx = self.switch_context(listed, package, record, langs).await?;
        Ok(render(package, &ctx, langs)?)
    }

    /// 重建用的上下文：局域网端口照记录开着的样子，显卡按记录生效。
    async fn switch_context(
        &self,
        listed: &Listed,
        package: &PluginPackage,
        record: &InstalledRecord,
        langs: &Languages,
    ) -> Result<InstallContext, InstallError> {
        let plan = self.addresses_for(listed, record.lan_webui, langs).await?;
        let addresses: Vec<_> = plan.bind.iter().map(|bound| bound.ip).collect();
        let grant = (!addresses.is_empty())
            .then(|| self.lan_grant_on(package, record.lan_webui, addresses, plan.announce));
        self.rebuild_context(package, record, grant, None).await
    }

    pub(in crate::plugin) fn listed(&self, id: &PluginId) -> Result<Listed, InstallError> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .get(id)
            .and_then(|entry| entry.listed.as_ref().map(|listed| (**listed).clone()))
            .ok_or_else(|| InstallError::NotInstalled(id.to_string()))
    }

    pub(in crate::plugin) fn package_of(
        &self,
        record: &InstalledRecord,
    ) -> Result<PluginPackage, InstallError> {
        let mut package = PluginPackage::with_files(
            &record.source,
            record.files.clone(),
            &self.owner_languages(),
        )?;
        package.origin = record.origin.clone();
        Ok(package)
    }

    pub(in crate::plugin) fn set_degraded(&self, id: &PluginId, reason: String) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(entry) = plugins.get_mut(id) {
            entry.status = PluginStatus::Degraded { reason };
        }
    }
}

fn changed_services(before: &Deployment, after: &Deployment) -> BTreeSet<String> {
    after
        .containers
        .iter()
        .filter(|container| {
            before
                .containers
                .iter()
                .find(|old| old.service == container.service)
                .is_none_or(|old| differs(old, container))
        })
        .map(|container| container.service.clone())
        .collect()
}

/// 变量集或容器定义（含挂载）不同就算变了。
fn differs(a: &ContainerPlan, b: &ContainerPlan) -> bool {
    if a.variables != b.variables {
        return true;
    }
    serde_json::to_value(&a.body).unwrap_or_default()
        != serde_json::to_value(&b.body).unwrap_or_default()
}
