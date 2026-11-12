//! 按官方索引的 `docs` 取一份文档：文件都按 sha256 核对，全部通过才切换，半途失败时旧的照常用。

use std::path::Path;

use crate::plugin::github_source::{SourceRef, Wanted};
use crate::plugin::manager::now_secs;
use crate::plugin::package_files::sha256_hex;
use crate::plugin::store::{create_private_dir, write_json};
use crate::plugin::storefront::index::{DocsSpec, Files};

use super::page;
use super::{CURRENT_FILE, Current, Source};
use crate::plugin::languages::Languages;

/// 一份文档最多这么多个文件：索引写错时不至于取个没完。
const MAX_FILES: usize = 500;

/// 取到 `cache/<提交号>/`，切换 `current.json`，删掉旧的。`reuse` 是手头那份文档的目录，内容没变的文件不重取。
pub(super) async fn fetch(
    spec: &DocsSpec,
    files: &Files,
    cache: &Path,
    reuse: Option<&Path>,
    langs: &Languages,
) -> Result<Current, String> {
    let source = SourceRef::parse(&spec.source, langs).map_err(|e| e.to_string())?;
    if source.wanted != Wanted::Latest {
        return Err("docs.source 不能带版本".to_owned());
    }
    if !crate::plugin::github::is_commit_id(&spec.commit) {
        return Err("docs.commit 要是 40 位提交号".to_owned());
    }
    if spec.files.len() > MAX_FILES {
        return Err(format!("docs.files 超过 {MAX_FILES} 个"));
    }
    if semver::Version::parse(&spec.min_core).is_err() {
        return Err(format!("docs.min_core 不是版本号：{}", spec.min_core));
    }
    let stage = cache.join(format!(".fetching-{}", spec.commit));
    remove_dir(&stage)?;
    create_private_dir(&stage).map_err(|e| e.to_string())?;
    let written = async {
        for (path, sha256) in &spec.files {
            if page::split_path(path).is_none() {
                return Err(format!("docs.files 里的路径不合法：{path}"));
            }
            let data = match reuse.and_then(|dir| unchanged(&dir.join(path), sha256)) {
                Some(data) => data,
                None => files
                    .verified(&source.repo, &spec.commit, path, sha256, langs)
                    .await
                    .map_err(|e| e.to_string())?,
            };
            let target = stage.join(path);
            if let Some(parent) = target.parent() {
                create_private_dir(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&target, data).map_err(|e| format!("{}：{e}", target.display()))?;
        }
        page::load_dir(&stage)
    }
    .await;
    let pages = match written {
        Ok(pages) if !pages.is_empty() => pages,
        Ok(_) => {
            let _ = std::fs::remove_dir_all(&stage);
            return Err("这份文档一页都没有".to_owned());
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(error);
        }
    };
    let dir = cache.join(&spec.commit);
    remove_dir(&dir)?;
    std::fs::rename(&stage, &dir).map_err(|e| format!("{}：{e}", dir.display()))?;
    let source = Source {
        commit: spec.commit.clone(),
        updated_at_ms: now_secs() * 1000,
        min_core: Some(spec.min_core.clone()),
    };
    write_json(&cache.join(CURRENT_FILE), &source).map_err(|e| e.to_string())?;
    prune(cache, &spec.commit);
    Ok(Current { source, dir, pages })
}

fn unchanged(path: &Path, sha256: &str) -> Option<Vec<u8>> {
    let data = std::fs::read(path).ok()?;
    (sha256_hex(&data) == sha256).then_some(data)
}

fn remove_dir(dir: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}：{e}", dir.display())),
    }
}

/// 删掉别的提交。删不掉只是多占点盘。
fn prune(cache: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(cache) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if entry.path().is_dir()
            && name != keep
            && let Err(error) = std::fs::remove_dir_all(entry.path())
        {
            tracing::warn!(dir = %entry.path().display(), %error, "旧的帮助文档删不掉");
        }
    }
}
