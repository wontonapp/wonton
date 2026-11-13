//! Installation outcomes are separate from installed entries: rolled-back providers must not satisfy dependencies.

use std::collections::BTreeMap;
use std::sync::Arc;

use wonton_protocol::{InstallFailure, InstallStage, InstalledPlugin};

use super::entry::{Entry, PluginStatus};
use super::languages::Languages;
use super::listed::Listed;
use super::manager::{InstallError, PluginManager};
use super::manifest::PluginId;

pub(super) struct FailedInstall {
    pub entry: Entry,
    pub failure: InstallFailure,
}

impl PluginManager {
    pub(super) fn install_stage(&self, id: &PluginId) -> InstallStage {
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .and_then(|entry| entry.install_progress.as_ref())
            .map_or(InstallStage::Preparing, |progress| progress.stage)
    }

    pub(super) fn installation_failed(
        &self,
        id: &PluginId,
        error: &InstallError,
        stage: InstallStage,
        cleanup_errors: Vec<String>,
        langs: &Languages,
    ) {
        let failure = InstallFailure {
            kind: error.kind().to_owned(),
            message: error.to_string(),
            stage,
            failed_plugin_id: id.to_string(),
            cleanup_errors,
        };
        self.retain_install_failure(id, failure, None, langs);
    }

    pub(super) fn retain_install_failure(
        &self,
        id: &PluginId,
        failure: InstallFailure,
        fallback: Option<Arc<Listed>>,
        langs: &Languages,
    ) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let mut failed = self.failed_installs.write().expect("安装结果锁中毒");
        let listed = plugins
            .remove(id)
            .and_then(|entry| entry.listed)
            .or_else(|| failed.remove(id).and_then(|entry| entry.entry.listed))
            .or(fallback);
        let Some(listed) = listed else { return };
        let reason = failure_reason(id, &failure, langs);
        trace_failure(id, &failure);
        failed.insert(
            id.clone(),
            FailedInstall {
                entry: Entry::new(listed, PluginStatus::Degraded { reason }, None),
                failure,
            },
        );
        drop(failed);
        drop(plugins);
        self.changed();
    }

    pub(super) fn failure_cause(&self, id: &PluginId, error: &InstallError) -> InstallFailure {
        let existing = self
            .failed_installs
            .read()
            .expect("安装结果锁中毒")
            .get(id)
            .map(|entry| entry.failure.clone());
        existing.unwrap_or_else(|| InstallFailure {
            kind: error.kind().to_owned(),
            message: error.to_string(),
            stage: self.install_stage(id),
            failed_plugin_id: id.to_string(),
            cleanup_errors: Vec::new(),
        })
    }

    pub(super) fn append_install_cleanup(
        &self,
        id: &PluginId,
        errors: &[String],
        langs: &Languages,
    ) {
        if errors.is_empty() {
            return;
        }
        let mut failed = self.failed_installs.write().expect("安装结果锁中毒");
        let Some(entry) = failed.get_mut(id) else {
            return;
        };
        entry.failure.cleanup_errors.extend_from_slice(errors);
        entry.entry.status = PluginStatus::Degraded {
            reason: failure_reason(id, &entry.failure, langs),
        };
        trace_failure(id, &entry.failure);
        drop(failed);
        self.changed();
    }

    pub(super) fn failed_install(&self, id: &PluginId) -> bool {
        self.failed_installs
            .read()
            .expect("安装结果锁中毒")
            .contains_key(id)
    }

    pub(super) fn failed_listing(
        &self,
        plugins: &BTreeMap<PluginId, Entry>,
        id: &PluginId,
        failed: &FailedInstall,
        langs: &Languages,
    ) -> InstalledPlugin {
        let mut plugin = self.listing(plugins, id, &failed.entry, langs);
        plugin.install_failure = Some(failed.failure.clone());
        plugin.install_progress = None;
        plugin.web_url = None;
        plugin.declares_tools = false;
        plugin.tabs.clear();
        plugin.tunnel_ports.clear();
        plugin.upgrade = None;
        plugin.notices.clear();
        plugin
    }

    pub(super) async fn clear_failed_install(
        &self,
        id: &PluginId,
        purge: bool,
        langs: &Languages,
    ) -> Result<super::manager::UninstallReport, InstallError> {
        let (needs_cleanup, hosted, storage) = {
            let failed = self.failed_installs.read().expect("安装结果锁中毒");
            let entry = failed
                .get(id)
                .ok_or_else(|| InstallError::not_installed(langs, id))?;
            let manifest = entry.entry.listed.as_ref().map(|listed| &listed.manifest);
            (
                !entry.failure.cleanup_errors.is_empty(),
                manifest.and_then(super::install_hosted::hosted_capability),
                manifest.is_some_and(|m| super::install_storage::storage_need(m).is_some()),
            )
        };
        let report = if purge || needs_cleanup {
            let result = self.teardown(id, purge, langs).await;
            if let Err(error) = &result {
                self.append_install_cleanup(id, &[error.to_string()], langs);
            }
            result?
        } else {
            super::manager::UninstallReport::default()
        };
        for capability in hosted
            .into_iter()
            .chain((purge && storage).then_some(super::capabilities::WEB_STORAGE.name))
        {
            if let Err(error) = self.hosted_dirs().remove(capability, id) {
                self.append_install_cleanup(id, &[error.to_string()], langs);
                return Err(InstallError::Storage(error.to_string()));
            }
        }
        self.failed_installs
            .write()
            .expect("安装结果锁中毒")
            .remove(id);
        self.changed();
        Ok(report)
    }
}

fn failure_reason(id: &PluginId, failure: &InstallFailure, langs: &Languages) -> String {
    let mut message = failure.message.clone();
    if id.as_str() != failure.failed_plugin_id {
        message = crate::l10n::text(
            langs,
            "plugin-settings-note-appended",
            &[
                ("message", message.into()),
                ("note", failure.failed_plugin_id.as_str().into()),
            ],
        );
    }
    if !failure.cleanup_errors.is_empty() {
        let note = crate::l10n::text(
            langs,
            "plugin-install-rollback-unfinished",
            &[(
                "failures",
                crate::l10n::list(langs, &failure.cleanup_errors).into(),
            )],
        );
        message = crate::l10n::text(
            langs,
            "plugin-settings-note-appended",
            &[("message", message.into()), ("note", note.into())],
        );
    }
    message
}

fn trace_failure(id: &PluginId, failure: &InstallFailure) {
    if failure.cleanup_errors.is_empty() {
        tracing::warn!(plugin = %id, stage = ?failure.stage, kind = %failure.kind,
            error = %failure.message, failed_plugin = %failure.failed_plugin_id,
            cleanup_errors = ?failure.cleanup_errors, "Plugin installation failed; rollback completed");
    } else {
        tracing::error!(plugin = %id, stage = ?failure.stage, kind = %failure.kind,
            error = %failure.message, failed_plugin = %failure.failed_plugin_id,
            cleanup_errors = ?failure.cleanup_errors, "Plugin installation failed; rollback incomplete");
    }
}
