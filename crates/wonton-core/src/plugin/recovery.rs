//! 已安装插件的运行意图与续建账本；Docker 请求超时不能证明操作没有发生。

use wonton_protocol::{PluginDesiredState, PluginLifecycle, PluginRecovery, PluginRecoveryStage};

use super::health_gate::Gate;
use super::manager::{InstallError, PluginManager, now_secs};
use super::manifest::PluginId;
use super::store::InstalledRecord;
use crate::l10n;

const RETRY_DELAYS: [u64; 5] = [30, 60, 120, 240, 300];

pub(super) fn running() -> PluginLifecycle {
    PluginLifecycle {
        desired: PluginDesiredState::Running,
        recovery: None,
    }
}

impl PluginManager {
    pub fn lifecycle_status(&self, id: &PluginId) -> Result<PluginLifecycle, InstallError> {
        Ok(self.recovery_record(id)?.lifecycle)
    }

    fn recovery_record(&self, id: &PluginId) -> Result<InstalledRecord, InstallError> {
        self.store
            .load(id)?
            .ok_or_else(|| InstallError::not_installed(&self.owner_languages(), id))
    }

    pub(super) fn explicitly_stopped(&self, id: &PluginId) -> Result<bool, InstallError> {
        Ok(self.lifecycle_status(id)?.desired == PluginDesiredState::Stopped)
    }

    pub(super) fn recovery_pending(&self, id: &PluginId) -> bool {
        self.lifecycle_status(id).is_ok_and(|lifecycle| {
            lifecycle.desired == PluginDesiredState::Running && lifecycle.recovery.is_some()
        })
    }

    pub(super) fn recovery_stage(
        &self,
        id: &PluginId,
        stage: PluginRecoveryStage,
    ) -> Result<(), InstallError> {
        let mut record = self.recovery_record(id)?;
        if record.lifecycle.desired == PluginDesiredState::Stopped {
            return Err(InstallError::Conflict(l10n::text(
                &self.owner_languages(),
                "plugin-recovery-owner-stopped",
                &[],
            )));
        }
        record.lifecycle.desired = PluginDesiredState::Running;
        let interrupted = self.interrupted(stage);
        let recovery = record
            .lifecycle
            .recovery
            .get_or_insert_with(|| interrupted.clone());
        if stage != PluginRecoveryStage::Preparing
            || !matches!(
                recovery.stage,
                PluginRecoveryStage::Replacing | PluginRecoveryStage::Starting
            )
        {
            recovery.stage = stage;
        }
        recovery.category = interrupted.category;
        recovery.reason = interrupted.reason;
        recovery.next_retry_at = None;
        self.store.save_lifecycle(id, record.lifecycle)?;
        self.changed();
        Ok(())
    }

    fn interrupted(&self, stage: PluginRecoveryStage) -> PluginRecovery {
        PluginRecovery {
            category: "interrupted".into(),
            reason: l10n::text(&self.owner_languages(), "plugin-recovery-interrupted", &[]),
            stage,
            attempts: 0,
            next_retry_at: None,
        }
    }

    pub(super) fn recovery_failed(
        &self,
        id: &PluginId,
        error: &InstallError,
    ) -> Result<(), InstallError> {
        let mut record = self.recovery_record(id)?;
        if record.lifecycle.desired == PluginDesiredState::Unspecified {
            return Ok(());
        }
        let recovery = record
            .lifecycle
            .recovery
            .get_or_insert_with(|| self.interrupted(PluginRecoveryStage::Preparing));
        recovery.category = error.kind().into();
        recovery.reason = error.to_string();
        recovery.next_retry_at = if record.lifecycle.desired == PluginDesiredState::Running {
            RETRY_DELAYS
                .get(recovery.attempts as usize)
                .map(|delay| now_secs().saturating_add(*delay))
        } else {
            None
        };
        self.store.save_lifecycle(id, record.lifecycle)?;
        self.changed();
        Ok(())
    }

    pub(super) fn recovery_complete(&self, id: &PluginId) -> Result<(), InstallError> {
        let mut record = self.recovery_record(id)?;
        if record.lifecycle.recovery.take().is_some() {
            self.store.save_lifecycle(id, record.lifecycle)?;
            self.changed();
        }
        Ok(())
    }

    /// 手动启动重新给额度；后台在任何 Docker 写操作前消费一次额度，崩溃也不会无限重试。
    pub(super) fn prepare_start(&self, id: &PluginId, manual: bool) -> Result<bool, InstallError> {
        let mut record = self
            .store
            .load(id)
            .map_err(|error| {
                InstallError::RecordUnusable(super::manager::unreadable_record(
                    &error,
                    &self.owner_languages(),
                ))
            })?
            .ok_or_else(|| InstallError::not_installed(&self.owner_languages(), id))?;
        let replacement = record.lifecycle.recovery.as_ref().is_some_and(|recovery| {
            matches!(
                recovery.stage,
                PluginRecoveryStage::Replacing | PluginRecoveryStage::Starting
            )
        });
        if manual {
            record.lifecycle.desired = PluginDesiredState::Running;
            let mut recovery = self.interrupted(PluginRecoveryStage::Preparing);
            if replacement {
                recovery.stage = PluginRecoveryStage::Replacing;
            }
            record.lifecycle.recovery = Some(recovery);
        } else {
            let recovery = record.lifecycle.recovery.as_mut().ok_or_else(|| {
                InstallError::Conflict(l10n::text(
                    &self.owner_languages(),
                    "plugin-recovery-not-due",
                    &[],
                ))
            })?;
            if record.lifecycle.desired != PluginDesiredState::Running
                || recovery.attempts as usize >= RETRY_DELAYS.len()
                || recovery.next_retry_at.is_none_or(|due| due > now_secs())
            {
                return Err(InstallError::Conflict(l10n::text(
                    &self.owner_languages(),
                    "plugin-recovery-not-due",
                    &[],
                )));
            }
            recovery.attempts += 1;
            recovery.next_retry_at = None;
        }
        self.store.save_lifecycle(id, record.lifecycle)?;
        self.changed();
        Ok(replacement)
    }

    pub(super) async fn retry_interrupted(
        &self,
        id: &PluginId,
        record: &InstalledRecord,
    ) -> Result<(), InstallError> {
        let Some(recovery) = &record.lifecycle.recovery else {
            return Ok(());
        };
        if record.lifecycle.desired != PluginDesiredState::Running
            || recovery.attempts as usize >= RETRY_DELAYS.len()
            || recovery.next_retry_at.is_none_or(|due| due > now_secs())
        {
            return Ok(());
        }
        self.start_with_gate(id, Gate::InstallTimeout, false)
            .await?;
        Ok(())
    }

    /// 上一进程的执行中日志转回排队；保留次数与最后阶段，不把异常归成普通 Stopped。
    pub(super) fn restore_recovery(
        &self,
        record: &InstalledRecord,
    ) -> Result<Option<String>, InstallError> {
        let Some(recovery) = &record.lifecycle.recovery else {
            return Ok(None);
        };
        if record.lifecycle.desired != PluginDesiredState::Running {
            return Ok(None);
        }
        let reason = recovery.reason.clone();
        if recovery.next_retry_at.is_none() && (recovery.attempts as usize) < RETRY_DELAYS.len() {
            let mut record = record.clone();
            record
                .lifecycle
                .recovery
                .as_mut()
                .expect("恢复日志存在")
                .next_retry_at =
                Some(now_secs().saturating_add(RETRY_DELAYS[recovery.attempts as usize]));
            self.store.save_lifecycle(&record.id, record.lifecycle)?;
        }
        Ok(Some(reason))
    }

    pub(super) fn queue_stopped(&self, id: &PluginId) -> Result<String, InstallError> {
        self.recovery_stage(id, PluginRecoveryStage::Reconciling)?;
        let error = InstallError::Unhealthy(l10n::text(
            &self.owner_languages(),
            "plugin-install-resident-stopped",
            &[],
        ));
        self.recovery_failed(id, &error)?;
        Ok(error.to_string())
    }
}
