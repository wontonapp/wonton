//! 配额（ADR-0113“配额与清理”）：任务进 `Preparing` 时看放不放得下，上传入库、下载写完时记账。
//!
//! 两本账：上传按提供方记，离线按设备记。放不下的任务在 `Preparing` 就失败，不等传到一半才被拒；
//! 失败码分两种，壳才知道该去清服务器（`server_storage_full`）还是清手机（`local_storage_full`）。

use wonton_protocol::media::{
    ItemState, MediaFailure, MediaFailureCode, MediaTask, TaskState, TaskTarget,
};

use super::store::{Ledger, MediaStoreError};
use super::task_service::{TaskService, TaskServiceError};

impl TaskService {
    /// 放不放得下这个任务还没传完的部分；放不下回要记到任务上的失败。上传记在 `provider` 那家；选不出提供方
    /// 时（`None`）不看上传账：调用方接着就把选不出的那个失败记到任务上，不会有字节走。
    pub(super) fn admit(
        &self,
        task: &MediaTask,
        provider: Option<&str>,
    ) -> Result<Option<MediaFailure>, TaskServiceError> {
        let (ledger, want) = match &task.target {
            TaskTarget::Upload(_) if provider.is_none() => return Ok(None),
            TaskTarget::Upload(items) => {
                let left = items
                    .iter()
                    .filter(|item| !matches!(item.state, ItemState::Done | ItemState::Skipped))
                    .map(|item| item.size)
                    .fold(0u64, u64::saturating_add);
                (Ledger::ProviderUpload, left)
            }
            // 下载多大要等提供方回了才知道；不知道时至少得还有一个字节的余量。
            TaskTarget::Download(_) => (
                Ledger::DeviceOffline,
                task.progress.total.unwrap_or(1).max(1),
            ),
            TaskTarget::Other(_) => return Ok(None),
        };
        let checked =
            self.store()
                .check_quota(&task.person, ledger, provider, Some(&task.device), want);
        match checked {
            Ok(()) => Ok(None),
            Err(exceeded @ MediaStoreError::QuotaExceeded { .. }) => Ok(Some(MediaFailure {
                code: full_code(ledger),
                message: exceeded.to_string(),
                retry_after_secs: None,
            })),
            Err(other) => Err(other.into()),
        }
    }

    /// 上传的一项入库了：按它的大小记到那家的上传账上。
    pub(super) fn charge_upload(
        &self,
        task: &MediaTask,
        provider: &str,
        bytes: u64,
    ) -> Result<(), TaskServiceError> {
        self.store().charge_quota(
            &task.person,
            Ledger::ProviderUpload,
            Some(provider),
            None,
            bytes,
        )?;
        Ok(())
    }

    /// 下载写完了：按写下的字节记到这台设备的离线账上，下一次 `PUT /offline` 再以设备报的为准。
    pub(super) fn charge_offline(&self, task: &MediaTask) -> Result<(), TaskServiceError> {
        debug_assert!(matches!(task.state, TaskState::Done));
        self.store().charge_quota(
            &task.person,
            Ledger::DeviceOffline,
            None,
            Some(&task.device),
            task.progress.done,
        )?;
        Ok(())
    }
}

fn full_code(ledger: Ledger) -> MediaFailureCode {
    match ledger {
        Ledger::ProviderUpload => MediaFailureCode::ServerStorageFull,
        Ledger::DeviceOffline => MediaFailureCode::LocalStorageFull,
    }
}
