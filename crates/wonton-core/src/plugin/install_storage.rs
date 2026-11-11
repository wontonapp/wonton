//! 静态网页的数据存储（ADR-0080 第 6 节）：插件在 `requires`/`wants` 里写了 `web_storage`，核心装的时候在
//! `hosted/web_storage/<插件 ID>/` 建一个属于主人用户的空目录，交给提供方读写。数据和容器插件的应用数据一样：
//! 升级不动，卸载默认留下，“彻底删除”才删。上限（每插件 64 MiB、1024 个文件、单文件 8 MiB）由提供方执行。

use super::capabilities::{STATIC_WEB, WEB_STORAGE};
use super::install_error::InstallError;
use super::languages::Languages;
use super::manager::PluginManager;
use super::manifest::{Manifest, PluginId, Runtime};
use crate::l10n;

/// 插件对数据存储的依赖。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StorageNeed {
    /// `requires`：没有提供方装不上，提供方停了插件标异常。
    Required,
    /// `wants`：没有提供方照装，数据路径回“没有就绪”。
    Wanted,
}

/// 清单对数据存储的依赖；只有静态网页能用它（ADR-0080 第 6 节的“使用方”）。
pub(super) fn storage_need(manifest: &Manifest) -> Option<StorageNeed> {
    if manifest.runtime != Runtime::StaticWeb {
        return None;
    }
    let named = |list: &[String]| list.iter().any(|name| name == WEB_STORAGE.name);
    if named(&manifest.requires) {
        Some(StorageNeed::Required)
    } else if named(&manifest.wants) {
        Some(StorageNeed::Wanted)
    } else {
        None
    }
}

/// 插件离不开的托管能力：静态网页的运行形态，加上 `requires` 里的数据存储。提供方停了这些插件就标异常，
/// 有它们在用时提供方卸不掉。
pub(super) fn required_hosted(manifest: &Manifest) -> Vec<&'static str> {
    let mut found = Vec::new();
    if manifest.runtime == Runtime::StaticWeb {
        found.push(STATIC_WEB.name);
    }
    if storage_need(manifest) == Some(StorageNeed::Required) {
        found.push(WEB_STORAGE.name);
    }
    found
}

impl PluginManager {
    /// 建好插件的数据目录（已经有就留着，里面是上次留下的数据）。返回这次是不是新建的：安装失败时只删新建的。
    pub(super) fn prepare_storage(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Result<bool, InstallError> {
        let dirs = self.hosted_dirs();
        let existed = dirs.exists(WEB_STORAGE.name, id);
        dirs.ensure_owned(WEB_STORAGE.name, id, self.owner())
            .map_err(|e| storage_error(langs, &e))?;
        Ok(!existed)
    }

    /// 没装着、却留着这个 ID 的数据目录（不删数据的卸载留下的）。
    pub(super) fn storage_retained(&self, id: &PluginId) -> bool {
        self.hosted_dirs().exists(WEB_STORAGE.name, id)
    }
}

pub(super) fn storage_error(langs: &Languages, error: &dyn std::fmt::Display) -> InstallError {
    InstallError::Storage(l10n::text(
        langs,
        "plugin-hosted-storage-failed",
        &[("error", error.to_string().into())],
    ))
}
