//! 升级失败后的收拾：旧容器没动过时收回备下的东西，换过容器后退回旧版本，成功后删新版本不再用的。

use super::super::bots::is_bot;
use super::super::health_gate::Gate;
use super::super::languages::Languages;
use super::super::lifecycle::client_spec;
use super::super::manager::{Deployed, InstallError, PluginManager};
use super::super::package::PluginPackage;
use super::super::render::render;
use super::super::upstreams::uses_sso;
use super::run::{Old, Progress, pin_images};
use crate::l10n;

impl PluginManager {
    /// 旧容器没动过：收回这次备下的，错误里写上收回失败的。
    pub(super) async fn undo_prepared(
        &self,
        old: &Old,
        progress: Progress,
        error: InstallError,
    ) -> InstallError {
        let langs = self.owner_languages();
        let id = &old.record.id;
        let mut failures = self.rollback(progress.undo, &langs).await;
        if let Err(e) = self
            .revoke_model_names(id, &progress.added_models, &langs)
            .await
        {
            failures.push(l10n::text(
                &langs,
                "plugin-settings-upgrade-cleanup-model-credentials",
                &[("error", e.to_string().into())],
            ));
        }
        if failures.is_empty() {
            error
        } else {
            error.with_note(
                &langs,
                &l10n::text(
                    &langs,
                    "plugin-settings-upgrade-cleanup-unfinished",
                    &[("failures", l10n::list(&langs, &failures).into())],
                ),
            )
        }
    }

    /// 换过容器之后失败：删掉新版本的一切，用旧记录和旧容器当时的镜像重建旧版本。
    pub(super) async fn back_to_old(
        &self,
        old: &Old,
        package: &PluginPackage,
        progress: Progress,
    ) -> Result<Deployed, InstallError> {
        let langs = self.owner_languages();
        let id = &old.record.id;
        let manifest = &old.package.manifest;
        let network = self.cfg.instance.network(&old.ctx.slug);
        self.remove_own_containers(id).await?;
        let failures = self.rollback(progress.undo, &langs).await;
        if !failures.is_empty() {
            tracing::error!(plugin = %id, ?failures, "收拾新版本的资源没做完");
        }
        if progress.network_rebuilt {
            self.rebuild_network(manifest, &network).await?;
        }
        self.reattach_core(&package.manifest, manifest, &network)
            .await?;
        self.register_source(manifest, &network).await?;
        self.revoke_model_names(id, &progress.added_models, &langs)
            .await?;
        let mut ctx = old.ctx.clone();
        ctx.core_address = self.core_address_var(manifest, &network).await?;
        ctx.extra_hosts = self.hosts_via_gateway(manifest, &network).await?;
        if let Some(hydra) = &self.oidc {
            match &old.record.oidc_client_secret {
                Some(secret) => {
                    hydra
                        .upsert_client(&client_spec(manifest, &ctx, secret))
                        .await?;
                }
                None if uses_sso(&package.manifest) => hydra.delete_client(id.as_str()).await?,
                None => {}
            }
        }
        self.sync_bot_web_clients(manifest, &ctx).await?;
        let mut plan = render(&old.package, &ctx, &langs)?;
        pin_images(&mut plan, &old.images);
        self.replace_containers(&ctx, plan, Gate::StartPeriod)
            .await?;
        // 对账要取宿主信息、起检查容器，链很深：同样放堆上。
        Box::pin(self.reconcile_running(&old.package, &old.record)).await
    }

    /// 新版本不要了的：记录已经写好，失败只记日志，主人卸载时一并清掉。
    pub(super) async fn drop_unused(&self, old: &Old, package: &PluginPackage, langs: &Languages) {
        let manifest = &package.manifest;
        let id = &manifest.id;
        let was = &old.package.manifest;
        if super::super::bot_web_registry::has_bot_web(was)
            && (!super::super::bot_web_registry::has_bot_web(manifest) || !uses_sso(manifest))
            && let Err(error) = self.drop_unused_bot_web(manifest).await
        {
            tracing::warn!(plugin = %id, %error, "Cannot delete unused Bot web registrations");
        }
        if let (true, false, Some(hydra)) = (uses_sso(was), uses_sso(manifest), &self.oidc)
            && let Err(e) = hydra.delete_client(id.as_str()).await
        {
            tracing::warn!(plugin = %id, error = %e, "删新版本不用的 OIDC 客户端失败");
        }
        if !super::super::models::uses_models(manifest)
            && let Err(e) = self.revoke_models(id, langs).await
        {
            tracing::warn!(plugin = %id, error = %e, "吊销新版本不用的模型网关凭据失败");
        }
        if is_bot(was)
            && !is_bot(manifest)
            && let Err(e) = self.deactivate_bots(id, false).await
        {
            tracing::warn!(plugin = %id, error = %e, "吊销新版本不用的 Bot 设备失败");
        }
        let lan_network = self.cfg.instance.lan_network(&id.slug());
        let used = self
            .lan_grant_on(package, false, Vec::new(), Vec::new())
            .network
            .is_some();
        if !used
            && self
                .engine
                .network_reach(&lan_network)
                .await
                .is_ok_and(|r| r.is_some())
            && let Err(e) = self.engine.remove_network(&lan_network).await
        {
            tracing::warn!(plugin = %id, error = %e, "删新版本不用的局域网网络失败");
        }
        if manifest
            .provides
            .as_ref()
            .is_some_and(|p| p.models.is_some())
        {
            for (plugin, error) in self.reconcile_models_locked().await {
                tracing::warn!(%plugin, %error, "模型网关升级后对账失败");
            }
        }
    }
}
