//! App 的缓存目录：系统会在空间不够时清掉、也不进备份。放取得回来的东西（头像、封面）。

use std::path::{Path, PathBuf};

/// 壳给的数据目录在 iOS 是 `Library/Application Support/<App>`，缓存放 `Library/Caches/<App>`；Android 是
/// `files`，缓存放同级的 `cache`。认不出来时放在数据目录下的 `cache` 里。
pub(crate) fn cache_root(data_dir: &Path) -> PathBuf {
    let parent = data_dir.parent();
    if let Some(support) =
        parent.filter(|path| path.file_name().is_some_and(|n| n == "Application Support"))
        && let (Some(library), Some(app)) = (support.parent(), data_dir.file_name())
    {
        return library.join("Caches").join(app);
    }
    if data_dir.file_name().is_some_and(|name| name == "files")
        && let Some(app) = parent
    {
        return app.join("cache");
    }
    data_dir.join("cache")
}
