//! 系统插件（`kind: system`，如模型网关）由安装器预装，API 装不了。核心启动时从 `WONTON_SYSTEM_PLUGINS` 目录装上
//! 还没装的系统插件，授予清单声明的全部权限：装核心时随附的系统插件，就是主人对它们的确认。已经装着的照常由
//! [`PluginManager::restore`] 核对，这里不重装、不升级。

use std::path::{Path, PathBuf};

use super::manager::{InstallError, PluginManager};
use super::manifest::PluginKind;
use super::package::PluginPackage;

impl PluginManager {
    /// 装上 `dir` 里还没装的系统插件，每个子目录一个插件包。单个插件装不上就把它标为异常、写明原因，不影响
    /// 核心启动；目录本身读不了时报错。
    pub async fn ensure_system(&self, dir: &Path) -> Result<(), InstallError> {
        let storage = |e: std::io::Error| {
            InstallError::Storage(format!("读取系统插件目录 {} 失败：{e}", dir.display()))
        };
        let mut packages: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(storage)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        packages.sort();
        for path in packages {
            let package = match PluginPackage::load_dir(&path) {
                Ok(package) => package,
                Err(e) => {
                    tracing::error!(path = %path.display(), error = %e, "系统插件包无法解析，没有安装");
                    continue;
                }
            };
            let manifest = &package.manifest;
            if manifest.kind != PluginKind::System {
                tracing::error!(plugin = %manifest.id, "系统插件目录里的插件不是 kind: system，没有安装");
                continue;
            }
            if self.is_installed(&manifest.id) {
                continue;
            }
            let (id, version) = (manifest.id.clone(), manifest.version.clone());
            let granted = manifest.permissions.clone();
            match self.install_checked(package, &granted).await {
                Ok(_) => tracing::info!(plugin = %id, "已预装系统插件"),
                Err(e) => {
                    tracing::error!(plugin = %id, error = %e, "预装系统插件失败");
                    self.mark_failed(&id, version, format!("预装失败：{e}"));
                }
            }
        }
        Ok(())
    }
}
