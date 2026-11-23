use super::super::manager::{InstallError, PluginManager};
use super::super::store::InstalledRecord;

impl PluginManager {
    pub(super) async fn save_upgraded_record(
        &self,
        mut record: InstalledRecord,
    ) -> Result<(), InstallError> {
        // 主人关入口不必等升级；提交时在开关共用的锁里保留最新状态，不能写回 read_old 的快照。
        let _pass = self.lan.pass.lock().await;
        let current = self
            .store
            .load(&record.id)?
            .ok_or_else(|| InstallError::not_installed(&self.owner_languages(), &record.id))?;
        record.entry_label = current.entry_label;
        record.clients_entries = current.clients_entries;
        record.setup_done_ever = current.setup_done_ever;
        self.store.save(&record)?;
        Ok(())
    }
}
