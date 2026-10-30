//! 插件目录：Bot 经 MCP 总线只能装这里的插件（ADR-0016）。M1 的目录是一个本地目录（`WONTON_CATALOG`），
//! 每个子目录一个插件包，格式同预装系统插件的目录；系统插件不在目录里。
//!
//! 每次查询都重新读目录：目录会随核心镜像或主人的操作变化，读一次只要几毫秒。

use std::path::{Path, PathBuf};

use super::manifest::{Manifest, PluginKind};
use super::package::PluginPackage;

#[derive(Debug, Clone)]
pub struct Catalog {
    dir: PathBuf,
}

impl Catalog {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// 目录里的全部插件包。读不了的子目录记一条错误后跳过，不影响其他插件。
    pub fn packages(&self) -> Result<Vec<PluginPackage>, String> {
        let entries = std::fs::read_dir(&self.dir)
            .map_err(|e| format!("读取插件目录 {} 失败：{e}", self.dir.display()))?;
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        dirs.sort();
        Ok(dirs.iter().filter_map(|dir| load(dir)).collect())
    }

    /// 按关键词找插件：每个词都出现在 ID、名字或说明里（不分大小写）。没有关键词时列出全部。
    pub fn search(&self, query: &str) -> Result<Vec<PluginPackage>, String> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut found = self.packages()?;
        found.retain(|package| {
            let text = searchable(&package.manifest);
            words.iter().all(|word| text.contains(word))
        });
        Ok(found)
    }

    /// 目录里 ID 为 `id` 的插件包。
    pub fn find(&self, id: &str) -> Result<Option<PluginPackage>, String> {
        Ok(self
            .packages()?
            .into_iter()
            .find(|package| package.manifest.id.as_str() == id))
    }
}

fn load(dir: &Path) -> Option<PluginPackage> {
    match PluginPackage::load_dir(dir) {
        Ok(package) if package.manifest.kind == PluginKind::System => None,
        Ok(package) => Some(package),
        Err(e) => {
            tracing::error!(path = %dir.display(), error = %e, "插件目录里的插件包无法解析，跳过");
            None
        }
    }
}

fn searchable(manifest: &Manifest) -> String {
    let texts = [&manifest.title, &manifest.description];
    let mut text = manifest.id.to_string();
    for value in texts.iter().flat_map(|t| t.values()) {
        text.push(' ');
        text.push_str(value);
    }
    text.to_lowercase()
}
