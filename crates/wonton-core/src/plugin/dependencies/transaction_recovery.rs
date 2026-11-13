use std::sync::Arc;

use super::{InstallError, Languages, Manifest, PluginManager, PluginStatus, declarations};
use crate::plugin::compose::PullPolicy;
use crate::plugin::entry::Entry;
use crate::plugin::health_gate::Gate;
use crate::plugin::listed::Listed;
use crate::plugin::package::PluginPackage;
use crate::plugin::render::{Deployment, InstallContext, render};
use crate::plugin::store::InstalledRecord;

pub(in crate::plugin) struct InstallConsumerSnapshot(Vec<ConsumerBefore>);

struct ConsumerBefore {
    listed: Arc<Listed>,
    record: InstalledRecord,
    package: PluginPackage,
    ctx: InstallContext,
    plan: Deployment,
    containers: Vec<Option<String>>,
}

impl PluginManager {
    /// Save the old variables before a new provider can connect or rebuild existing consumers.
    pub(in crate::plugin) async fn snapshot_install_consumers(
        &self,
        manifests: &[&Manifest],
        langs: &Languages,
    ) -> Result<InstallConsumerSnapshot, InstallError> {
        let mut consumers = Vec::new();
        for listed in self.capability_consumers() {
            if !declarations(&listed.manifest).any(|capability| {
                manifests.iter().any(|manifest| {
                    manifest
                        .provides
                        .as_ref()
                        .is_some_and(|provides| provides.capabilities.contains_key(capability))
                })
            }) {
                continue;
            }
            let id = &listed.manifest.id;
            if !matches!(
                self.status_of(id),
                Some(PluginStatus::Running | PluginStatus::Degraded { .. })
            ) {
                continue;
            }
            let record = self
                .store
                .load(id)?
                .ok_or_else(|| InstallError::not_installed(langs, id))?;
            let package = crate::plugin::upgrade::installed_package(&record, langs)?;
            let lan = self.lan_grant(&package, record.lan_webui, langs).await?;
            let mut ctx = self.rebuild_context(&package, &record, lan, None).await?;
            self.fill_bot_vars(&package, &mut ctx, langs).await?;
            let mut plan = render(&package, &ctx, langs)?;
            let mut containers = Vec::with_capacity(plan.containers.len());
            for container in &mut plan.containers {
                let info = self.engine.inspect_container(&container.name).await?;
                if let Some(info) = &info {
                    if let Some(image) = &info.image {
                        container.image.clone_from(image);
                        container.body.image = Some(image.clone());
                        container.pull_policy = PullPolicy::Missing;
                    }
                    if let Some(config) = &info.config {
                        container.body.env.clone_from(&config.env);
                    }
                }
                containers.push(info.and_then(|info| info.id));
            }
            consumers.push(ConsumerBefore {
                listed,
                record,
                package,
                ctx,
                plan,
                containers,
            });
        }
        Ok(InstallConsumerSnapshot(consumers))
    }

    /// Dependency State must already be restored; old rendered variables never use revoked keys.
    pub(in crate::plugin) async fn restore_install_consumers(
        &self,
        snapshot: InstallConsumerSnapshot,
        langs: &Languages,
    ) -> Vec<String> {
        let mut failures = Vec::new();
        for before in snapshot.0.into_iter().rev() {
            let id = before.record.id.clone();
            if let Err(error) = self.restore_install_consumer(before).await {
                let detail = error.with_note(langs, id.as_str()).to_string();
                let mut plugins = self.plugins.write().expect("插件表锁中毒");
                if let Some(entry) = plugins.get_mut(&id)
                    && let Some(listed) = entry.listed.clone()
                {
                    *entry = Entry::containers_failed(listed, detail.clone());
                }
                failures.push(detail);
            }
        }
        self.changed();
        failures
    }

    async fn restore_install_consumer(&self, before: ConsumerBefore) -> Result<(), InstallError> {
        let mut changed = false;
        for (container, original_id) in before.plan.containers.iter().zip(&before.containers) {
            let current = self.engine.inspect_container(&container.name).await?;
            changed |= current.as_ref().and_then(|info| info.id.as_ref()) != original_id.as_ref()
                || current
                    .as_ref()
                    .and_then(|info| info.config.as_ref())
                    .and_then(|config| config.env.as_ref())
                    != container.body.env.as_ref();
        }
        if !changed {
            return Ok(());
        }
        self.replace_containers(&before.ctx, before.plan, Gate::StartPeriod)
            .await?;
        let deployed = self
            .reconcile_running(&before.package, &before.record)
            .await?;
        if let Some(problem) = &deployed.problem {
            return Err(InstallError::UpstreamUnavailable(problem.clone()));
        }
        self.store.save_metadata(&before.record)?;
        self.plugins
            .write()
            .expect("插件表锁中毒")
            .insert(before.record.id, Entry::running(before.listed, deployed));
        Ok(())
    }
}
