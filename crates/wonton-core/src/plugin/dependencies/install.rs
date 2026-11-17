use std::sync::Arc;

use wonton_protocol::InstallRequest;

use super::preflight::PreparedInstall;
use super::{InstallError, Languages, PluginManager};
use crate::plugin::install_failure::RetryInstall;
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
        detached(async move {
            let _changing = this.dependencies.changing.lock().await;
            this.install_dependency_sequence(package, options, None, None)
                .await
        })
        .await
    }

    pub(crate) async fn retry_failed_install(
        self: &Arc<Self>,
        id: PluginId,
    ) -> Result<PluginView, InstallError> {
        let this = Arc::clone(self);
        detached(async move {
            let _changing = this.dependencies.changing.lock().await;
            let langs = this.owner_languages();
            let (retry, previous) = {
                let failed = this.failed_installs.read().expect("安装结果锁中毒");
                let entry = failed
                    .get(&id)
                    .filter(|entry| entry.failure.retryable)
                    .ok_or_else(|| {
                        InstallError::Conflict(crate::l10n::text(
                            &langs,
                            "plugin-settings-retry-unavailable",
                            &[],
                        ))
                    })?;
                (
                    entry.retry.clone().expect("可重试的安装有原始决定"),
                    entry.failure.clone(),
                )
            };
            let digest = retry.package.digest();
            let result = this
                .install_dependency_sequence(
                    retry.package,
                    DependencyInstall {
                        request: retry.request,
                        langs,
                    },
                    Some(retry.providers),
                    Some(&previous),
                )
                .await;
            if result.is_ok() {
                this.staged.remove(&digest);
            }
            result
        })
        .await
    }

    async fn install_dependency_sequence(
        &self,
        package: PluginPackage,
        options: DependencyInstall,
        retained: Option<Vec<PluginPackage>>,
        previous: Option<&wonton_protocol::InstallFailure>,
    ) -> Result<PluginView, InstallError> {
        let mut retry = RetryInstall {
            package: package.clone(),
            request: options.request.clone(),
            providers: Vec::new(),
        };
        let (layers, snapshot, consumers) = self
            .prepare_dependency_batch(&package, &options, retained.as_deref(), previous)
            .await?;
        retry.providers = layers
            .iter()
            .take(layers.len().saturating_sub(1))
            .map(|layer| layer.decision.package.clone())
            .collect();
        // 预检已过、开始逐层安装：把根插件旧失败条目标成重试中，客户端断线后的等待逻辑据此
        // 不把服务器上还没替换的旧失败记录当成这次的结果。
        if previous.is_some() {
            self.mark_retrying(&retry.package.manifest.id);
        }
        self.install_batch_layers(layers, &options, &retry, snapshot, consumers)
            .await
    }

    /// 预检整批安装：不在升级中、计划可满足，快照能力与使用方状态。重试预检失败时把旧失败记录改成这次的原因。
    async fn prepare_dependency_batch(
        &self,
        package: &PluginPackage,
        options: &DependencyInstall,
        retained: Option<&[PluginPackage]>,
        previous: Option<&wonton_protocol::InstallFailure>,
    ) -> Result<
        (
            Vec<PreparedInstall>,
            super::plan::CapabilityStateSnapshot,
            super::transaction_recovery::InstallConsumerSnapshot,
        ),
        InstallError,
    > {
        let langs = &options.langs;
        let preflight = async {
            self.check_not_upgrading(langs)?;
            let layers = self
                .prepare_install_batch(package.clone(), options.request.clone(), retained, langs)
                .await?;
            let snapshot = self.snapshot_capability_state()?;
            let manifests = layers
                .iter()
                .map(|layer| &layer.decision.package.manifest)
                .collect::<Vec<_>>();
            let consumers = self.snapshot_install_consumers(&manifests, langs).await?;
            Ok::<_, InstallError>((layers, snapshot, consumers))
        }
        .await;
        match preflight {
            Ok(prepared) => Ok(prepared),
            Err(error) => {
                if let Some(previous) = previous {
                    self.retry_preflight_failed(&package.manifest.id, previous, &error, langs);
                }
                Err(error)
            }
        }
    }

    /// 逐层安装计划里的包并核对能力授予；任何一层失败，把整批交回 [`Self::rollback_dependency_batch`]。
    async fn install_batch_layers(
        &self,
        layers: Vec<PreparedInstall>,
        options: &DependencyInstall,
        retry: &RetryInstall,
        snapshot: super::plan::CapabilityStateSnapshot,
        consumers: super::transaction_recovery::InstallConsumerSnapshot,
    ) -> Result<PluginView, InstallError> {
        let langs = &options.langs;
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
            let layer_retry = (manifest.id == retry.package.manifest.id).then_some(retry);
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
                self.install_checked_retained(&decision, settings, layer_retry, langs)
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
                        self.retain_install_failure_with_retry(
                            &manifest.id,
                            cause.clone(),
                            Some(Listed::new(
                                &decision.package,
                                crate::plugin::manager::now_secs(),
                                decision.lan_webui,
                                false,
                                Default::default(),
                            )),
                            layer_retry.cloned(),
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
                            retry.clone(),
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
                    .rollback_dependency_batch(
                        rollback,
                        snapshot,
                        consumers,
                        (error, cause),
                        retry.clone(),
                        langs,
                    )
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
        retry: RetryInstall,
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
            let layer_retry = (layer.id == retry.package.manifest.id).then_some(&retry);
            if let Some(undo) = layer.undo {
                failures.extend(
                    self.rollback_installed(&layer.id, undo, &root, layer_retry, langs)
                        .await,
                );
            } else {
                self.retain_install_failure_with_retry(
                    &layer.id,
                    root,
                    None,
                    layer_retry.cloned(),
                    langs,
                );
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
        let root_id = &retry.package.manifest.id;
        if !self.is_installed(root_id) {
            let mut failure = cause;
            failure.cleanup_errors.extend(failures.iter().cloned());
            self.retain_install_failure_with_retry(
                root_id,
                failure,
                Some(Listed::new(
                    &retry.package,
                    crate::plugin::manager::now_secs(),
                    retry.request.lan_webui,
                    false,
                    Default::default(),
                )),
                Some(retry.clone()),
                langs,
            );
        } else {
            self.append_install_cleanup(root_id, &failures, langs);
        }
        self.install_rollback_error(error, failures, langs)
    }
}
