//! 系统插件（`kind: system`，如模型网关）由安装器预装，API 装不了。核心启动时从 `WONTON_SYSTEM_PLUGINS` 目录装上
//! 还没装的系统插件，装不上时怎样重试见 [`super::preinstall`]。已经装着的照常由 [`PluginManager::restore`] 核对，
//! 这里不重装；目录里的版本比装着的新时升级它（见 [`super::upgrade`]）。
//!
//! 例外是记录读不懂的：核心升级后不再认旧记录里的清单写法时，升级要先读旧包，走不通；系统插件又不能经接口重装，
//! 主人在 App 里没有出路。这时保留数据卸掉，用目录里的包重新装上（和主人“卸载（保留数据）再装”一样）。

use std::path::Path;

use super::manager::{InstallError, PluginManager};
use super::manifest::{PluginId, PluginKind};
use super::package::PluginPackage;
use super::preinstall::Backoff;

impl PluginManager {
    /// 装上 `dir` 里还没装的系统插件，每个子目录一个插件包，一个装完（或停下）再装下一个。单个插件装不上不影响核心
    /// 启动；目录本身读不了时报错。
    pub async fn ensure_system(&self, dir: &Path, backoff: Backoff) -> Result<(), InstallError> {
        for package in self.bundled_in(dir)? {
            let manifest = &package.manifest;
            if manifest.kind != PluginKind::System {
                tracing::error!(plugin = %manifest.id, "系统插件目录里的插件不是 kind: system，没有安装");
                continue;
            }
            if self.is_installed(&manifest.id) {
                if self.record_unreadable(&manifest.id) {
                    self.reinstall_bundled(package, backoff).await;
                } else {
                    self.upgrade_bundled(package).await;
                }
                continue;
            }
            self.preinstall(package, backoff).await;
        }
        Ok(())
    }

    /// 记录在，但里面的插件包这份核心读不懂。记录文件本身读不了（磁盘、权限）不算：那不是换个包能修好的。
    fn record_unreadable(&self, id: &PluginId) -> bool {
        match self.store.load(id) {
            Ok(Some(record)) => {
                PluginPackage::with_files(&record.source, record.files, &self.owner_languages())
                    .is_err()
            }
            Ok(None) | Err(_) => false,
        }
    }

    async fn reinstall_bundled(&self, package: PluginPackage, backoff: Backoff) {
        let id = package.manifest.id.clone();
        tracing::warn!(plugin = %id, "系统插件的记录读不懂，保留数据用随核心带来的包重装");
        if let Err(e) = self.uninstall(&id, false, &self.owner_languages()).await {
            tracing::error!(plugin = %id, error = %e, "卸掉读不懂的系统插件失败，没有重装");
            return;
        }
        self.preinstall(package, backoff).await;
    }
}
