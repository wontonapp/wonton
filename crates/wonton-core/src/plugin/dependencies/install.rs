use std::sync::Arc;

use wonton_protocol::InstallRequest;

use super::preflight::PreparedInstall;
use super::{InstallError, Languages, PluginManager};
use crate::plugin::lifecycle::Undo;
use crate::plugin::listed::Listed;
use crate::plugin::manager::{PluginView, detached};
use crate::plugin::manifest::PluginId;
use crate::plugin::package::PluginPackage;
use crate::plugin::secrets::Secrets;

pub(crate) struct DependencyInstall {
    pub request: InstallRequest,
    pub langs: Languages,
}

struct LayerRollback {
    id: PluginId,
    secrets: Option<Secrets>,
    changed: std::collections::BTreeMap<String, u64>,
    undo: Option<Undo>,
}

impl PluginManager {
    pub(crate) async fn install_with_dependencies(
        self: &Arc<Self>,
        package: PluginPackage,
        options: DependencyInstall,
    ) -> Result<PluginView, InstallError> {
        let this = Arc::clone(self);
        detached(async move { this.install_dependency_sequence(package, options).await }).await
    }

    async fn install_dependency_sequence(
        &self,
        package: PluginPackage,
        options: DependencyInstall,
    ) -> Result<PluginView, InstallError> {
        let _changing = self.dependencies.changing.lock().await;
        let langs = &options.langs;
        self.check_not_upgrading(langs)?;
        let layers = self
            .prepare_install_batch(package, options.request.clone(), langs)
            .await?;
        let snapshot = self.snapshot_capability_state()?;
        let manifests = layers
            .iter()
            .map(|layer| &layer.decision.package.manifest)
            .collect::<Vec<_>>();
        let consumers = self.snapshot_install_consumers(&manifests, langs).await?;
        let mut rollback = Vec::with_capacity(layers.len());
        let mut manifests = Vec::with_capacity(layers.len());
        let mut result = None;
        for PreparedInstall {
            decision,
            inputs,
            settings,
            secrets_before,
            changed_before,
        } in layers
        {
            let manifest = decision.package.manifest.clone();
            self.failed_installs
                .write()
                .expect("安装结果锁中毒")
                .remove(&manifest.id);
            rollback.push(LayerRollback {
                id: manifest.id.clone(),
                secrets: secrets_before,
                changed: changed_before,
                undo: None,
            });
            let installed = async {
                self.store_input_values(&manifest, &inputs, None, langs)?;
                self.install_checked_retained(&decision, settings, langs)
                    .await
            }
            .await;
            match installed {
                Ok((view, undo)) => {
                    rollback.last_mut().expect("安装层已记录").undo = Some(undo);
                    manifests.push(manifest);
                    result = Some(view);
                }
                Err(error) => {
                    let cause = self.failure_cause(&manifest.id, &error);
                    if !self.failed_install(&manifest.id) && !self.is_installed(&manifest.id) {
                        self.retain_install_failure(
                            &manifest.id,
                            cause.clone(),
                            Some(Listed::new(
                                &decision.package,
                                crate::plugin::manager::now_secs(),
                                decision.lan_webui,
                                false,
                                Default::default(),
                            )),
                            langs,
                        );
                    }
                    let error = error.with_note(langs, manifest.id.as_str());
                    return Err(self
                        .rollback_dependency_batch(
                            rollback,
                            snapshot,
                            consumers,
                            (error, cause),
                            langs,
                        )
                        .await);
                }
            }
        }
        for manifest in &manifests {
            let connect = if manifests
                .last()
                .is_some_and(|consumer| consumer.id == manifest.id)
            {
                options.request.connect_plugins.as_slice()
            } else {
                &[]
            };
            if let Err(error) = self.capabilities_installed(manifest, connect, false).await {
                let cause = self.failure_cause(&manifest.id, &error);
                return Err(self
                    .rollback_dependency_batch(rollback, snapshot, consumers, (error, cause), langs)
                    .await);
            }
        }
        for manifest in &manifests {
            self.probe_fresh_consumer(manifest).await;
        }
        Ok(result.expect("安装计划包含使用方"))
    }

    async fn rollback_dependency_batch(
        &self,
        rollback: Vec<LayerRollback>,
        snapshot: super::plan::CapabilityStateSnapshot,
        consumers: super::transaction_recovery::InstallConsumerSnapshot,
        (error, cause): (InstallError, wonton_protocol::InstallFailure),
        langs: &Languages,
    ) -> InstallError {
        let mut failures = Vec::new();
        if let Err(error) = self.restore_capability_state(snapshot, langs).await {
            failures.push(error.to_string());
        }
        failures.extend(Box::pin(self.restore_install_consumers(consumers, langs)).await);
        let shared_failures = failures.clone();
        for layer in rollback.into_iter().rev() {
            let mut root = cause.clone();
            root.cleanup_errors.extend(shared_failures.iter().cloned());
            if let Some(undo) = layer.undo {
                failures.extend(self.rollback_installed(&layer.id, undo, &root, langs).await);
            } else {
                self.retain_install_failure(&layer.id, root, None, langs);
            }
            let restored = match layer.secrets {
                Some(secrets) => {
                    self.store
                        .secrets()
                        .save_changed(&layer.id, &secrets, &layer.changed)
                }
                None => self.store.secrets().remove(&layer.id),
            };
            if let Err(error) = restored {
                failures.push(error.to_string());
                self.append_install_cleanup(&layer.id, &[error.to_string()], langs);
            }
        }
        self.install_rollback_error(error, failures, langs)
    }
}
