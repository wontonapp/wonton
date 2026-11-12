//! 托管形态的目录（ADR-0080 第 2 节）：`<核心数据目录>/hosted/<能力名>/` 整个挂进提供方的容器，
//! `<能力名>/<插件 ID>/` 是一个插件的根。核心只改目录里的内容，不重建提供方的容器（Docker 不能给运行中的容器加挂载）。
//!
//! 换内容是原子的：先写进 `.staging/<插件 ID>-<摘要>/`，再用 `renameat2(RENAME_EXCHANGE)` 和 `<插件 ID>/` 对换，
//! 进行中的请求看到的要么全是旧版本、要么全是新版本。插件 ID 只有小写字母、数字、连字符和点（`PluginId::parse`），
//! 不会是 `..`，也撞不上 `.staging`。

use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use rustix::fs::{CWD, RenameFlags, renameat_with};

use super::hostdir::{self, Owner};
use super::manifest::PluginId;
use super::package::static_files::StaticFiles;

/// 解包中的目录，提供方的配置不能把它当成插件根（插件 ID 不以 `.` 开头）。
const STAGING: &str = ".staging";

/// 核心数据目录下的 `hosted/`。
#[derive(Debug, Clone)]
pub struct HostedDirs {
    root: PathBuf,
}

/// 写好、还没对换进去的新内容。丢弃时删掉。
#[must_use]
pub struct Staged {
    path: PathBuf,
}

/// 对换进去了：`old` 是换下来的旧内容（第一次装时没有）。记录保存成功再 [`Swapped::finish`]，失败就 [`Swapped::undo`]。
#[must_use]
pub struct Swapped {
    live: PathBuf,
    old: Option<PathBuf>,
}

impl HostedDirs {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            root: data_dir.join("hosted"),
        }
    }

    /// `hosted/` 本身。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 一项能力的总目录，挂进提供方的容器。
    pub fn capability_dir(&self, capability: &str) -> PathBuf {
        self.root.join(capability)
    }

    /// 一个插件在这项能力下的根。
    pub fn plugin_dir(&self, capability: &str, id: &PluginId) -> PathBuf {
        self.capability_dir(capability).join(id.as_str())
    }

    /// 总目录在不在都建好：提供方的容器挂它之前要有，否则 Docker 以 root 建一个空目录。
    pub fn ensure_capability(&self, capability: &str) -> io::Result<PathBuf> {
        let dir = self.capability_dir(capability);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(&dir)?;
        Ok(dir)
    }

    /// 插件的根在不在。
    pub fn exists(&self, capability: &str, id: &PluginId) -> bool {
        fs::symlink_metadata(self.plugin_dir(capability, id)).is_ok_and(|meta| meta.is_dir())
    }

    /// 把文件写进解包目录。目录 0755、文件 0644：提供方以主人的用户只读挂载，读得到就行。
    pub fn stage(
        &self,
        capability: &str,
        id: &PluginId,
        digest: &str,
        files: &StaticFiles,
    ) -> io::Result<Staged> {
        let staging = self.ensure_capability(capability)?.join(STAGING);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&staging)?;
        let short = digest.trim_start_matches("sha256:");
        let path = staging.join(format!("{id}-{}", &short[..short.len().min(16)]));
        remove_tree(&path)?;
        let staged = Staged { path };
        fs::DirBuilder::new().mode(0o755).create(&staged.path)?;
        for (rel, data) in files {
            let file = staged.path.join(rel);
            if let Some(parent) = file.parent() {
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o755)
                    .create(parent)?;
            }
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o644)
                .open(&file)
                .and_then(|mut out| io::Write::write_all(&mut out, data))?;
        }
        Ok(staged)
    }

    /// 换上解包好的内容：已有旧内容时原子对换，没有时直接改名。
    pub fn swap_in(&self, capability: &str, id: &PluginId, staged: Staged) -> io::Result<Swapped> {
        let live = self.plugin_dir(capability, id);
        let staged_path = staged.path.clone();
        let old = match fs::symlink_metadata(&live) {
            Ok(_) => {
                renameat_with(CWD, &staged_path, CWD, &live, RenameFlags::EXCHANGE)?;
                Some(staged_path)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::rename(&staged_path, &live)?;
                None
            }
            Err(e) => return Err(e),
        };
        std::mem::forget(staged);
        Ok(Swapped { live, old })
    }

    /// 给插件建一个属于主人用户的空根（可写的托管能力，如 `web_storage`）。已有就留着：里面是插件的数据。
    pub fn ensure_owned(
        &self,
        capability: &str,
        id: &PluginId,
        owner: Owner,
    ) -> Result<PathBuf, hostdir::DirError> {
        let dir = self.plugin_dir(capability, id);
        self.ensure_capability(capability)
            .map_err(|source| hostdir::DirError::Io {
                path: self.capability_dir(capability),
                source,
            })?;
        hostdir::ensure_owned_dir(&dir, owner)?;
        Ok(dir)
    }

    /// 删掉插件的根；本来就没有不算错。
    pub fn remove(&self, capability: &str, id: &PluginId) -> io::Result<()> {
        remove_tree(&self.plugin_dir(capability, id))
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if let Err(e) = remove_tree(&self.path) {
            tracing::warn!(dir = %self.path.display(), error = %e, "删不掉没用上的解包目录");
        }
    }
}

impl Swapped {
    /// 记录保存好了：删掉换下来的旧内容。删不掉只记日志，下次同一摘要解包前会再删。
    pub fn finish(self) {
        if let Some(old) = &self.old
            && let Err(e) = remove_tree(old)
        {
            tracing::warn!(dir = %old.display(), error = %e, "删不掉换下来的旧静态文件");
        }
    }

    /// 记录没存上：换回旧内容（第一次装的直接删掉新内容）。
    pub fn undo(self) -> io::Result<()> {
        match &self.old {
            Some(old) => {
                renameat_with(CWD, old, CWD, &self.live, RenameFlags::EXCHANGE)?;
                remove_tree(old)
            }
            None => remove_tree(&self.live),
        }
    }
}

fn remove_tree(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> PluginId {
        PluginId::parse(text).unwrap()
    }

    fn files(pairs: &[(&str, &str)]) -> StaticFiles {
        pairs
            .iter()
            .map(|(path, data)| ((*path).to_owned(), data.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn a_new_version_replaces_the_old_one_whole_and_undo_puts_it_back() {
        let data = tempfile::tempdir().unwrap();
        let dirs = HostedDirs::new(data.path());
        let snake = id("io.github.alice.snake");
        let first = files(&[("index.html", "v1"), ("js/old.js", "old")]);
        let staged = dirs
            .stage("static_web", &snake, "sha256:aa", &first)
            .unwrap();
        dirs.swap_in("static_web", &snake, staged).unwrap().finish();
        let live = dirs.plugin_dir("static_web", &snake);
        assert_eq!(fs::read_to_string(live.join("index.html")).unwrap(), "v1");

        let second = files(&[("index.html", "v2")]);
        let staged = dirs
            .stage("static_web", &snake, "sha256:bb", &second)
            .unwrap();
        let swapped = dirs.swap_in("static_web", &snake, staged).unwrap();
        assert_eq!(fs::read_to_string(live.join("index.html")).unwrap(), "v2");
        assert!(
            !live.join("js/old.js").exists(),
            "旧版本的文件不留在新版本里"
        );
        swapped.undo().unwrap();
        assert_eq!(fs::read_to_string(live.join("index.html")).unwrap(), "v1");

        let staging = dirs.capability_dir("static_web").join(STAGING);
        assert_eq!(
            fs::read_dir(&staging).unwrap().count(),
            0,
            "解包目录不留东西"
        );
        dirs.remove("static_web", &snake).unwrap();
        assert!(!dirs.exists("static_web", &snake));
        dirs.remove("static_web", &snake).unwrap();
    }

    #[test]
    fn a_dropped_stage_leaves_nothing_behind() {
        let data = tempfile::tempdir().unwrap();
        let dirs = HostedDirs::new(data.path());
        let snake = id("io.github.alice.snake");
        drop(
            dirs.stage(
                "static_web",
                &snake,
                "sha256:cc",
                &files(&[("index.html", "x")]),
            )
            .unwrap(),
        );
        assert!(!dirs.exists("static_web", &snake));
        let staging = dirs.capability_dir("static_web").join(STAGING);
        assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
    }
}
