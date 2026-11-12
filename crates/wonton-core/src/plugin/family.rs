//! 插件记录里的 `family` 开关及其设置接口；应用访问权限由角色的 `open_apps` 决定（ADR-0088）。

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

    pub fn family_enabled_checked(&self, id: &PluginId) -> Result<bool, super::store::StoreError> {
        Ok(self.store.load(id)?.is_some_and(|record| record.family))
    }

    /// 修改插件记录的 `family` 标记，返回改后的插件；不改变角色授权。
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
