//! 插件的“家人能用”开关（ADR-0045）：插件全家一份，主人开放了家人才用得到。开关在插件记录里，跟着插件
//! 走升级和重建。

use wonton_protocol::InstalledPlugin;

use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::manifest::PluginId;
use super::store::InstalledRecord;

impl PluginManager {
    /// 家人能不能用这个插件：主人开放了才行（默认关）。
    pub fn family_enabled(&self, id: &PluginId) -> bool {
        self.store
            .load(id)
            .ok()
            .flatten()
            .is_some_and(|record| record.family)
    }

    /// 主人设一个插件给不给家人用，返回改后的插件。
    pub fn set_family(
        &self,
        id: &PluginId,
        enabled: bool,
        langs: &Languages,
    ) -> Result<InstalledPlugin, InstallError> {
        let mut record: InstalledRecord = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::NotInstalled(id.to_string()))?;
        record.family = enabled;
        self.store.save(&record)?;
        self.installed_plugin(id, langs)
            .ok_or_else(|| InstallError::Storage(format!("插件 {id} 的详情构造不出来")))
    }
}
