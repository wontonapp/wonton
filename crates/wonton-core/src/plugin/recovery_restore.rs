//! 核心重启只恢复意图与账本，不把尚未完成的重建误当成主人主动停止。

use wonton_protocol::PluginDesiredState;

use super::entry::Entry;
use super::health_gate::Gate;
use super::listed::Listed;
use super::manager::{InstallError, PluginManager};
use super::package::PluginPackage;
use super::store::InstalledRecord;

impl PluginManager {
    pub(super) async fn restore_runtime(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
    ) -> Result<Entry, InstallError> {
        let listed = Listed::of_record(package, record);
        if record.lifecycle.desired == PluginDesiredState::Stopped {
            return Ok(match &record.lifecycle.recovery {
                Some(recovery) => Entry::containers_failed(listed, recovery.reason.clone()),
                None => Entry::stopped(listed),
            });
        }
        if let Some(reason) = self.restore_recovery(record)? {
            return Ok(Entry::containers_failed(listed, reason));
        }
        match self.reconcile(package, record, Gate::InstallTimeout).await {
            Ok(Some(deployed)) => {
                if deployed.problem.is_some() && !listed.open_ports(record.lan_webui).is_empty() {
                    self.lan.mark_degraded(&record.id);
                }
                if let Some(reason) = &deployed.problem {
                    if self.recovery_pending(&record.id) {
                        self.recovery_failed(&record.id, &InstallError::Unhealthy(reason.clone()))?;
                    }
                } else {
                    self.recovery_complete(&record.id)?;
                }
                Ok(Entry::running(listed, deployed))
            }
            Ok(None) if record.lifecycle.desired == PluginDesiredState::Running => {
                let reason = self.queue_stopped(&record.id)?;
                Ok(Entry::containers_failed(listed, reason))
            }
            Ok(None) => Ok(Entry::stopped(listed)),
            Err(error) => {
                if self.recovery_pending(&record.id) {
                    self.recovery_failed(&record.id, &error)?;
                } else if !listed.open_ports(record.lan_webui).is_empty() {
                    // 纯地址异常仍由局域网检测核对，不绕开原有的网络选择。
                    self.lan.mark_degraded(&record.id);
                }
                Ok(Entry::containers_failed(listed, error.to_string()))
            }
        }
    }
}
