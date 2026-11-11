//! `plugin check-index`：官方索引里的每一项这个版本的核心都读得懂、渲染得出来。CI 用它当门禁，改了注入的
//! 变量或清单写法、索引跟不上时当场失败。
//!
//! 渲染走的是安装前的 [`dry_run`]，和 `plugin check` 同一条路。索引里只有文件的 sha256、没有内容，所以被引用的
//! 文件按路径填占位内容：渲染只看得到文件在不在，看不到图标格式、表单内容这些（`plugin check` 的活）。

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use wonton_core::plugin::capabilities;
use wonton_core::plugin::github_source::SourceRef;
use wonton_core::plugin::languages::Languages;
use wonton_core::plugin::package::PluginPackage;
use wonton_core::plugin::package_files::COMPOSE_FILE;
use wonton_core::plugin::render::dry_run;
use wonton_core::plugin::storefront::index::{Index, IndexEntry};

/// 被引用文件的占位内容：`notices` 要能当文本读，所以取合法 UTF-8。
const PLACEHOLDER: &[u8] = b"wonton check-index\n";

pub(super) fn run(path: &Path) -> ExitCode {
    let parsed = std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|json| Index::parse(&json, &Languages::default()).map_err(|e| e.to_string()));
    let index = match parsed {
        Ok(index) => index,
        Err(e) => {
            eprintln!("读不了 {}：{e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let problems = problems(&index);
    for problem in &problems {
        eprintln!("{problem}");
    }
    if !problems.is_empty() {
        return ExitCode::FAILURE;
    }
    println!("{} 个插件都读得懂、渲染得出来", index.plugins.len());
    ExitCode::SUCCESS
}

fn problems(index: &Index) -> Vec<String> {
    let mut problems: Vec<String> = index
        .skipped
        .iter()
        .map(|plugin| format!("读不懂 {}：{}", plugin.source, plugin.reason))
        .collect();
    problems.extend(uncovered(index));
    problems.extend(colliding_subdirs(index));
    for listed in &index.plugins {
        if let Err(reason) = dry_render(&listed.entry) {
            problems.push(format!("{}：{reason}", listed.entry.source));
        }
    }
    problems
}

/// 同一仓库里两个子目录的最后一段相同：它们的 tag 前缀会撞（规范的“插件仓库”）。
/// [`Index::parse`] 只逐项看来源，看不出这一对；`check-index` 看得到整份索引，在这里把关。
fn colliding_subdirs(index: &Index) -> Vec<String> {
    let langs = Languages::default();
    let mut seen: BTreeMap<(String, String), &str> = BTreeMap::new();
    let mut problems = Vec::new();
    for listed in &index.plugins {
        // 读不懂的来源已经在 skipped 里报过。
        let Ok(source) = SourceRef::parse(&listed.entry.source, &langs) else {
            continue;
        };
        let Some(last) = source
            .subdir
            .as_deref()
            .and_then(|subdir| subdir.rsplit('/').next())
        else {
            continue;
        };
        let key = (source.repo.to_string(), last.to_owned());
        match seen.get(&key) {
            Some(previous) => problems.push(format!(
                "同一仓库的子目录最后一段重复：{previous} 和 {}，tag 前缀 {last}/v 会撞上",
                listed.entry.source
            )),
            None => {
                seen.insert(key, listed.entry.source.as_str());
            }
        }
    }
    problems
}

/// ADR-0053：每个被 requires 的能力，索引里至少要有一个插件提供，否则这个使用方装不上。
fn uncovered(index: &Index) -> Vec<String> {
    let provides = |name: &str| {
        index
            .plugins
            .iter()
            .any(|listed| match capabilities::lookup(name) {
                Some(capability) => capabilities::is_provided(capability, &listed.manifest),
                // 第三方能力核心只按名字配对：有人声明提供就算数。
                None => listed
                    .manifest
                    .provides
                    .as_ref()
                    .is_some_and(|p| p.capabilities.contains_key(name)),
            })
    };
    index
        .plugins
        .iter()
        .flat_map(|listed| {
            listed
                .manifest
                .requires
                .iter()
                .filter(|name| !provides(name))
                .map(|name| {
                    format!(
                        "{} 需要能力 {name}，索引里没有插件提供它",
                        listed.entry.source
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// 按索引条目渲染一遍，和安装前一样，只是被引用的文件只有占位内容。
fn dry_render(entry: &IndexEntry) -> Result<(), String> {
    let langs = Languages::default();
    let files = entry
        .files
        .keys()
        .filter(|path| path.as_str() != COMPOSE_FILE)
        .map(|path| (path.clone(), PLACEHOLDER.to_vec()))
        .collect();
    let package =
        PluginPackage::with_files(&entry.compose, files, &langs).map_err(|e| e.to_string())?;
    dry_run(&package, &langs)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
