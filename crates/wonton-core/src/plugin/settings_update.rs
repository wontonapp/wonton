//! 读写插件设置（ADR-0077）：校验值、存进记录，写成功后只重建渲染结果里用到它的服务，失败退回原值。
//!
//! 和改一项密钥（[`super::secret_update`]）同一条路：同一把重建锁、同样的回滚，只是值存进记录、可以读出来。

use std::collections::{BTreeMap, BTreeSet};

use super::health_gate::Gate;
use super::install_error::InstallError;
use super::languages::Languages;
use super::listed::Listed;
use super::manager::{PluginManager, detached, now_secs};
use super::manifest::PluginId;
use super::package::PluginPackage;
use super::render::render;
use super::settings::{self, Schema, Values};
use super::store::InstalledRecord;
use std::sync::Arc;
use wonton_protocol::{PluginSettingsUpdate, PluginSettingsView};

impl PluginManager {
    /// 读设置页：Schema 不合法时同样报错，值照存着的样子给。
    pub(crate) async fn plugin_settings(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Result<PluginSettingsView, InstallError> {
        let record = self.record(id)?;
        let package = self.package_of(&record)?;
        let schema = self.settings_schema(&package, langs)?;
        let services = self
            .settings_services(&record, &package, &schema, langs)
            .await?;
        Ok(settings::view(
            &schema,
            &record.settings,
            &services,
            record.settings_changed_ms,
            langs,
        ))
    }

    /// 只校验、不保存：返回填上候选值的视图，问题在字段上。
    pub(crate) async fn preview_plugin_settings(
        &self,
        id: &PluginId,
        update: &PluginSettingsUpdate,
        langs: &Languages,
    ) -> Result<PluginSettingsView, InstallError> {
        let record = self.record(id)?;
        let package = self.package_of(&record)?;
        let schema = self.settings_schema(&package, langs)?;
        self.check_digest(&schema, &update.schema_digest)?;
        let values = settings::value::merged(&schema, &record.settings, &update.values, langs)
            .map_err(|fields| settings::value_error(fields, langs))?;
        let services = self
            .settings_services(&record, &package, &schema, langs)
            .await?;
        let mut view = settings::view(
            &schema,
            &values,
            &services,
            record.settings_changed_ms,
            langs,
        );
        // 预览里当前值的问题来自候选值；原来的存值不动。
        for (field, problem) in settings::value::problems(&values, langs, &schema) {
            if let Some(item) = view.form.fields.iter_mut().find(|item| item.name == field) {
                item.problem = Some(problem);
            }
        }
        Ok(view)
    }

    /// 保存设置。校验通过才改记录；用完到的服务重建，失败退回原值。
    pub(crate) async fn set_plugin_settings(
        self: &Arc<Self>,
        id: &PluginId,
        update: &PluginSettingsUpdate,
        langs: &Languages,
    ) -> Result<PluginSettingsView, InstallError> {
        let (this, id, update, langs) = (self.clone(), id.clone(), update.clone(), langs.clone());
        detached(async move { this.set_plugin_settings_inner(&id, &update, &langs).await }).await
    }

    async fn set_plugin_settings_inner(
        &self,
        id: &PluginId,
        update: &PluginSettingsUpdate,
        langs: &Languages,
    ) -> Result<PluginSettingsView, InstallError> {
        let _pass = self.lan.pass.lock().await;
        let listed = self.listed(id)?;
        let record = self.record(id)?;
        let package = self.package_of(&record)?;
        let schema = self.settings_schema(&package, langs)?;
        self.check_digest(&schema, &update.schema_digest)?;
        let values = settings::value::merged(&schema, &record.settings, &update.values, langs)
            .map_err(|fields| settings::value_error(fields, langs))?;
        if values == record.settings {
            return self
                .judged_settings(&record, &package, &schema, &values, langs)
                .await;
        }
        if !self.begin_rebuild(id) {
            return Err(InstallError::Conflict(format!(
                "插件 {id} 正在安装、卸载或重建，等那边做完"
            )));
        }
        let before = record.clone();
        let mut after = record.clone();
        after.settings = values.clone();
        after.settings_changed_ms = Some(now_secs() * 1000);
        let result = self
            .apply_settings(&listed, &package, &before, &after, langs)
            .await;
        if let Err(e) = &result {
            // 值退回原样再照旧渲染一次：重建可能已经删了旧容器或起了一半新的。
            match self
                .apply_settings(&listed, &package, &after, &before, langs)
                .await
            {
                Ok(()) => self.changed(),
                Err(back) => {
                    tracing::error!(plugin = %id, error = %back, "改设置失败，退回旧值也失败");
                    self.set_degraded(id, format!("改设置失败，退回也失败：{back}"));
                }
            }
            tracing::warn!(plugin = %id, error = %e, "改设置失败");
            self.lan.end(id);
            return Err(InstallError::SettingsInvalid(e.to_string(), Vec::new()));
        }
        tracing::info!(plugin = %id, "主人改了一项设置");
        self.changed();
        self.lan.end(id);
        let record = self.record(id)?;
        self.judged_settings(&record, &package, &schema, &record.settings, langs)
            .await
    }

    /// 按 `from`、`to` 各渲染一次，只替换渲染结果变了的服务，成功后存下 `to`。
    async fn apply_settings(
        &self,
        listed: &Listed,
        package: &PluginPackage,
        from: &InstalledRecord,
        to: &InstalledRecord,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let before = self
            .render_for_settings(listed, package, from, langs)
            .await?;
        let after = self.render_for_settings(listed, package, to, langs).await?;
        let changed = changed_services(&before, &after);
        if changed.is_empty() {
            self.store.save(to)?;
            return Ok(());
        }
        let mut plan = after;
        plan.retain_services(&changed);
        let ctx = self.settings_context(listed, package, to, langs).await?;
        self.replace_containers(&ctx, plan, Gate::StartPeriod)
            .await?;
        self.store.save(to)?;
        Ok(())
    }

    async fn render_for_settings(
        &self,
        listed: &Listed,
        package: &PluginPackage,
        record: &InstalledRecord,
        langs: &Languages,
    ) -> Result<super::render::Deployment, InstallError> {
        let ctx = self
            .settings_context(listed, package, record, langs)
            .await?;
        Ok(render(package, &ctx, langs)?)
    }

    async fn settings_context(
        &self,
        listed: &Listed,
        package: &PluginPackage,
        record: &InstalledRecord,
        langs: &Languages,
    ) -> Result<super::render::InstallContext, InstallError> {
        let plan = self.addresses_for(listed, record.lan_webui, langs).await?;
        let addresses: Vec<_> = plan.bind.iter().map(|bound| bound.ip).collect();
        let grant = (!addresses.is_empty())
            .then(|| self.lan_grant_on(package, record.lan_webui, addresses, plan.announce));
        let mut ctx = self.rebuild_context(package, record, grant, None).await?;
        self.fill_bot_vars(package, &mut ctx, langs).await?;
        Ok(ctx)
    }

    fn record(&self, id: &PluginId) -> Result<InstalledRecord, InstallError> {
        self.installed_record(id)?
            .ok_or_else(|| InstallError::NotInstalled(id.to_string()))
    }

    fn settings_schema(
        &self,
        package: &PluginPackage,
        langs: &Languages,
    ) -> Result<Schema, InstallError> {
        settings::load(package, langs)?
            .ok_or_else(|| InstallError::Unsupported("这个版本的应用没有设置".to_owned()))
    }

    fn check_digest(&self, schema: &Schema, digest: &str) -> Result<(), InstallError> {
        if schema.digest != digest {
            return Err(InstallError::Changed(format!(
                "设置文件在读取之后换了（现在是 {}），重新读一遍",
                schema.digest
            )));
        }
        Ok(())
    }

    /// 每个设置被哪些服务引用：照存着的值渲染一次，只看变量表。
    async fn settings_services(
        &self,
        record: &InstalledRecord,
        package: &PluginPackage,
        schema: &Schema,
        langs: &Languages,
    ) -> Result<BTreeMap<String, Vec<String>>, InstallError> {
        if schema.fields.is_empty() {
            return Ok(BTreeMap::new());
        }
        let listed = match self.plugins.read().expect("插件表锁中毒").get(&record.id) {
            Some(entry) => entry.listed.clone(),
            None => return Ok(BTreeMap::new()),
        };
        let Some(listed) = listed else {
            return Ok(BTreeMap::new());
        };
        let plan = self
            .render_for_settings(&listed, package, record, langs)
            .await?;
        Ok(settings::services(schema, &plan.containers))
    }

    async fn judged_settings(
        &self,
        record: &InstalledRecord,
        package: &PluginPackage,
        schema: &Schema,
        values: &Values,
        langs: &Languages,
    ) -> Result<PluginSettingsView, InstallError> {
        let services = self
            .settings_services(record, package, schema, langs)
            .await?;
        Ok(settings::view(
            schema,
            values,
            &services,
            record.settings_changed_ms,
            langs,
        ))
    }
}

/// 两次渲染之间容器用的变量或计划变了，就是渲染结果变了的服务。
fn changed_services(
    before: &super::render::Deployment,
    after: &super::render::Deployment,
) -> BTreeSet<String> {
    let mut changed = BTreeSet::new();
    for container in &after.containers {
        let old = before
            .containers
            .iter()
            .find(|old| old.service == container.service);
        let same = old.is_some_and(|old| {
            old.variables == container.variables
                && super::render_drift::digest(old) == super::render_drift::digest(container)
        });
        if !same {
            changed.insert(container.service.clone());
        }
    }
    changed
}

impl PluginManager {
    /// 卡片上的设置表单：安装用默认值，升级用装着的值并标出问题。
    pub(crate) fn card_settings(
        &self,
        package: &PluginPackage,
        installed: Option<&InstalledRecord>,
        langs: &Languages,
    ) -> Result<Option<wonton_protocol::SettingsForm>, InstallError> {
        let Some(schema) = settings::load(package, langs)? else {
            return Ok(None);
        };
        let mut values: Values = installed
            .map(|record| record.settings.clone())
            .unwrap_or_default();
        settings::value::defaults(&schema, &mut values);
        Ok(Some(settings::value::form(&schema, &values, langs)))
    }
}
