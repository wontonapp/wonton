//! 系统插件（`kind: system`，如模型网关）由安装器预装，API 装不了。核心启动时从 `WONTON_SYSTEM_PLUGINS` 目录装上
//! 还没装的系统插件，装不上时怎样重试见 [`super::preinstall`]。已经装着的照常由 [`PluginManager::restore`] 核对，
//! 这里不重装；目录里的版本比装着的新时升级它（见 [`super::upgrade`]）。

use std::path::Path;

use super::manager::{InstallError, PluginManager};
use super::manifest::PluginKind;
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
                self.upgrade_bundled(package).await;
                continue;
            }
            self.preinstall(package, backoff).await;
        }
        Ok(())
    }
}
