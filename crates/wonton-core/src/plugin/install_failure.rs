//! Installation outcomes are separate from installed entries: rolled-back providers must not satisfy dependencies.

use std::collections::BTreeMap;
use std::sync::Arc;

use wonton_protocol::{InstallFailure, InstallRequest, InstallStage, InstalledPlugin, PluginState};

use super::entry::{Entry, PluginStatus};
use super::languages::Languages;
use super::listed::Listed;
use super::manager::{InstallError, PluginManager};
use super::manifest::PluginId;
use super::package::PluginPackage;

#[derive(Clone)]
pub(super) struct RetryInstall {
    pub package: PluginPackage,
    pub request: InstallRequest,
    pub providers: Vec<PluginPackage>,
}

pub(super) struct FailedInstall {
    pub entry: Entry,
    pub failure: InstallFailure,
    pub retry: Option<RetryInstall>,
    /// 重试已过预检、正在逐层安装：列表把它显示成安装中，不再当失败（客户端断线后的
    /// 等待逻辑靠它区分“还在装”和“这次又失败了”）。任何写回/删除都会换掉整条记录，标记随之消失。
    pub retrying: bool,
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
        retry: Option<RetryInstall>,
        langs: &Languages,
    ) {
        let failure = InstallFailure {
            kind: error.kind().to_owned(),
            message: error.to_string(),
            stage,
            failed_plugin_id: id.to_string(),
            cleanup_errors,
            retryable: false,
        };
        self.retain_install_failure_with_retry(id, failure, None, retry, langs);
    }

    pub(super) fn retain_install_failure_with_retry(
        &self,
        id: &PluginId,
        mut failure: InstallFailure,
        fallback: Option<Arc<Listed>>,
        retry: Option<RetryInstall>,
        langs: &Languages,
    ) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let mut failed = self.failed_installs.write().expect("安装结果锁中毒");
        let previous = failed.remove(id);
        let listed = plugins
            .remove(id)
            .and_then(|entry| entry.listed)
            .or_else(|| {
                previous
                    .as_ref()
                    .and_then(|entry| entry.entry.listed.clone())
            })
            .or(fallback);
        let retry = retry.or_else(|| previous.and_then(|entry| entry.retry));
        let Some(listed) = listed else { return };
        failure.retryable = retry.is_some() && failure.cleanup_errors.is_empty();
        let reason = failure_reason(id, &failure, langs);
        trace_failure(id, &failure);
        failed.insert(
            id.clone(),
            FailedInstall {
                entry: Entry::new(listed, PluginStatus::Degraded { reason }, None),
                failure,
                retry,
                retrying: false,
            },
        );
        drop(failed);
        drop(plugins);
        self.changed();
    }

    pub(in crate::plugin) fn retry_preflight_failed(
        &self,
        id: &PluginId,
        previous: &InstallFailure,
        error: &InstallError,
        langs: &Languages,
    ) {
        let mut failed = self.failed_installs.write().expect("安装结果锁中毒");
        let Some(entry) = failed
            .get_mut(id)
            .filter(|entry| &entry.failure == previous)
        else {
            return;
        };
        entry.failure.kind = error.kind().to_owned();
        entry.failure.message = error.to_string();
        entry.failure.stage = InstallStage::Preparing;
        entry.failure.failed_plugin_id = id.to_string();
        if matches!(
            error,
            InstallError::Changed(_) | InstallError::NotFound(_) | InstallError::Invalid(_)
        ) {
            entry.failure.retryable = false;
            entry.retry = None;
        }
        entry.entry.status = PluginStatus::Degraded {
            reason: failure_reason(id, &entry.failure, langs),
        };
        drop(failed);
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
            retryable: false,
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
        entry.failure.retryable = false;
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

    /// 重试预检通过、开始逐层安装：根插件的旧失败条目标成重试中，列表照安装中显示、不带旧失败。
    /// 不删整条记录：删了列表里就没有这个插件，客户端断线重连后会按 MISSING_LIMIT 判成不确定。
    pub(super) fn mark_retrying(&self, id: &PluginId) {
        if let Some(entry) = self
            .failed_installs
            .write()
            .expect("安装结果锁中毒")
            .get_mut(id)
        {
            entry.retrying = true;
        }
        self.changed();
    }

    pub(super) fn failed_listing(
        &self,
        plugins: &BTreeMap<PluginId, Entry>,
        id: &PluginId,
        failed: &FailedInstall,
        langs: &Languages,
    ) -> InstalledPlugin {
        let mut plugin = self.listing(plugins, id, &failed.entry, langs);
        // 重试正在逐层安装：照安装中显示，不带旧失败，客户端断线后的等待逻辑据此不判失败。
        if failed.retrying {
            plugin.status = PluginState::Installing;
        } else {
            plugin.install_failure = Some(failed.failure.clone());
        }
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
        let (needs_cleanup, remote, hosted, storage) = {
            let failed = self.failed_installs.read().expect("安装结果锁中毒");
            let entry = failed
                .get(id)
                .ok_or_else(|| InstallError::not_installed(langs, id))?;
            let manifest = entry.entry.listed.as_ref().map(|listed| &listed.manifest);
            (
                !entry.failure.cleanup_errors.is_empty(),
                manifest.is_some_and(|m| m.remote_mcp.is_some()),
                manifest.and_then(super::install_hosted::hosted_capability),
                manifest.is_some_and(|m| super::install_storage::storage_need(m).is_some()),
            )
        };
        let mut report = if purge || needs_cleanup {
            let result = self.teardown(id, purge, langs).await;
            if let Err(error) = &result {
                self.append_install_cleanup(id, &[error.to_string()], langs);
            }
            result?
        } else {
            super::manager::UninstallReport::default()
        };
        // 探针可能已经在连接器留下会话和令牌；失败安装也要撤，不能只删本机记录。
        if remote {
            report
                .warnings
                .extend(self.forget_connector(id, langs).await);
        }
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
        self.mcp_unavailable
            .write()
            .expect("远端 MCP 状态锁中毒")
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
