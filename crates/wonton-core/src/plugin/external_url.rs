//! 规范对外地址的注入和开关变化后的重建（ADR-0089 第 5 节、ADR-0077）。
//!
//! 值本身怎么算在 [`crate::clients::external_url`]；这里只接上安装上下文和只重建引用它的服务。

use std::collections::BTreeSet;

use super::health_gate::Gate;
use super::install_error::InstallError;
use super::manager::PluginManager;
use super::manifest::PluginId;
use super::render::render;
use super::store::InstalledRecord;
use crate::clients::external_url;

impl PluginManager {
    /// 安装、重建时注入的值。记录还没写时按全关算；插件没有网页时用同一条隧道地址当退路
    /// （`WONTON_APP_URL` 只在有网页时才有）。
    pub(in crate::plugin) async fn external_url_value(
        &self,
        id: &PluginId,
        app_url: Option<&str>,
    ) -> Result<String, InstallError> {
        let record = self.store.load(id)?;
        let suffix = match self
            .clients_gateway()
            .and_then(|gateway| gateway.domain.clone())
        {
            Some(domain) => domain
                .suffix()
                .map_err(|error| InstallError::Storage(error.to_string()))?,
            None => None,
        };
        let fallback = app_url.map(str::to_owned).unwrap_or_else(|| {
            let public = &self.cfg.public;
            public.origin(&public.plugin_host(&id.slug()))
        });
        Ok(external_url::value(
            record
                .as_ref()
                .map(|record| record.clients_entries)
                .unwrap_or_default(),
            record
                .as_ref()
                .and_then(|record| record.entry_label.as_deref()),
            suffix.as_deref(),
            Some(&fallback),
        )
        .unwrap_or(fallback))
    }

    /// 开关改了规范对外地址：只重建 compose 里引用它的服务，改动和渲染走的是同一条重建管线。
    /// 返回值是这次有没有服务被重启（没有服务引用它时是 `false`）。
    pub(crate) async fn rebuild_external_url_users(
        &self,
        record: &InstalledRecord,
    ) -> Result<bool, InstallError> {
        let package = self.package_of(record)?;
        if !package.references_external_url() {
            return Ok(false);
        }
        let langs = self.owner_languages();
        let listed = self.listed(&record.id)?;
        let plan = self
            .addresses_for(&listed, record.lan_webui, &langs)
            .await?;
        let addresses: Vec<_> = plan.bind.iter().map(|bound| bound.ip).collect();
        let grant = (!addresses.is_empty())
            .then(|| self.lan_grant_on(&package, record.lan_webui, addresses, plan.announce));
        let mut ctx = self.rebuild_context(&package, record, grant, None).await?;
        self.fill_bot_vars(&package, &mut ctx, &langs).await?;
        let mut rendered = render(&package, &ctx, &langs)?;
        let users: BTreeSet<String> = rendered
            .containers
            .iter()
            .filter(|container| container.variables.contains(external_url::VAR))
            .map(|container| container.service.clone())
            .collect();
        if users.is_empty() {
            // 引用的可能是插值进不了容器变量的位置：值已经存下，下次启动生效。
            return Ok(false);
        }
        rendered.retain_services(&users);
        self.replace_containers(&ctx, rendered, Gate::StartPeriod)
            .await?;
        self.recovery_complete(&record.id)?;
        Ok(true)
    }
}
