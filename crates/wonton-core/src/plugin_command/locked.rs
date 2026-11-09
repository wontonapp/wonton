use std::collections::BTreeSet;
use std::path::Path;
use std::process::ExitCode;

use wonton_core::plugin::PluginPackage;
use wonton_core::plugin::egress::{address::ProxyAddress, core_proxy::CoreProxy};
use wonton_core::plugin::github::{GitHub, is_commit_id};
use wonton_core::plugin::github_source::{self, SourceRef, Wanted};
use wonton_core::plugin::languages::Languages;
use wonton_core::plugin::manifest::PluginKind;
use wonton_core::plugin::package_files::COMPOSE_FILE;

struct Entry {
    category: String,
    name: String,
    source: SourceRef,
    commit: String,
    digest: String,
}

fn parse(text: &str) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    let mut names = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        let invalid = |why: &str| format!("锁文件第 {} 行：{why}", index + 1);
        let mut fields = line.split_whitespace();
        let (Some(category), Some(name), Some(source), Some(commit), Some(digest), None) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            return Err(invalid(
                "须为 <system|default> <名称> <github:来源@版本> <提交号> <sha256:摘要>",
            ));
        };
        if !matches!(category, "system" | "default") {
            return Err(invalid("类别须为 system 或 default"));
        }
        if name.len() > 63
            || name.split('-').any(|part| {
                part.is_empty()
                    || !part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            })
        {
            return Err(invalid("名称须为小写字母、数字和单个连字符，最长 63 字节"));
        }
        if !names.insert(name) {
            return Err(invalid("名称重复"));
        }
        if !is_commit_id(commit) {
            return Err(invalid("提交号须为 40 位小写十六进制"));
        }
        let source =
            SourceRef::parse(source, &Languages::default()).map_err(|e| invalid(&e.to_string()))?;
        match &source.wanted {
            Wanted::Latest => return Err(invalid("来源必须明确指定 tag 或 40 位提交号")),
            Wanted::Commit(wanted) if wanted != commit => {
                return Err(invalid("来源的提交号和锁定提交不一致"));
            }
            _ => {}
        }
        let hash = digest.strip_prefix("sha256:").unwrap_or_default();
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(invalid("摘要须为 sha256: 加 64 位小写十六进制"));
        }
        entries.push(Entry {
            category: category.to_owned(),
            name: name.to_owned(),
            source,
            commit: commit.to_owned(),
            digest: digest.to_owned(),
        });
    }
    if entries.is_empty() {
        return Err("锁文件没有插件".to_owned());
    }
    Ok(entries)
}

pub(super) async fn run(lock: &Path, destination: &Path) -> ExitCode {
    let result = async {
        let text = std::fs::read_to_string(lock).map_err(|e| format!("{}：{e}", lock.display()))?;
        let entries = parse(&text)?;
        let github = github_from_env()?;
        std::fs::create_dir(destination).map_err(|e| {
            format!(
                "创建 {} 失败（目标目录必须不存在）：{e}",
                destination.display()
            )
        })?;
        let result = fetch_all(&github, &entries, destination).await;
        if result.is_err()
            && let Err(error) = std::fs::remove_dir_all(destination)
        {
            eprintln!("清理 {} 失败：{error}", destination.display());
        }
        result
    }
    .await;
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("取锁定插件失败：{error}");
            ExitCode::FAILURE
        }
    }
}

// 构建阶段没有主人的持久设置；只在这个 CLI 中读构建工具传入的标准 HTTPS 代理变量。
fn github_from_env() -> Result<GitHub, String> {
    let proxy = CoreProxy::default();
    if let Some(value) = ["https_proxy", "HTTPS_PROXY"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
    {
        proxy.set(Some(&ProxyAddress::parse(&value, &Languages::default())?));
    }
    Ok(GitHub::new(&proxy))
}

async fn fetch_all(github: &GitHub, entries: &[Entry], destination: &Path) -> Result<(), String> {
    for category in ["system", "default"] {
        std::fs::create_dir(destination.join(format!("{category}-plugins")))
            .map_err(|e| e.to_string())?;
    }
    for entry in entries {
        let origin = github_source::resolve(github, &entry.source, &Languages::default())
            .await
            .map_err(|e| e.to_string())?;
        if origin.commit != entry.commit {
            return Err(format!(
                "{}：来源解析为提交 {}，锁定的是 {}",
                entry.source.text(),
                origin.commit,
                entry.commit
            ));
        }
        let package = github_source::fetch(github, &origin, &Languages::default())
            .await
            .map_err(|e| e.to_string())?;
        check_package(entry, &package)?;
        let dir = destination
            .join(format!("{}-plugins", entry.category))
            .join(&entry.name);
        write_package(&package, &dir)?;
        println!(
            "{} {} {} {}",
            entry.category, entry.name, entry.commit, entry.digest
        );
    }
    Ok(())
}

fn check_package(entry: &Entry, package: &PluginPackage) -> Result<(), String> {
    if (entry.category == "system") != (package.manifest.kind == PluginKind::System) {
        return Err(format!(
            "{}：锁文件类别 {} 和清单 kind 不一致",
            entry.name, entry.category
        ));
    }
    let digest = package.digest();
    if digest != entry.digest {
        return Err(format!(
            "{}：插件包摘要 {digest} 和锁定的 {} 不一致",
            entry.name, entry.digest
        ));
    }
    Ok(())
}

fn write_package(package: &PluginPackage, dir: &Path) -> Result<(), String> {
    std::fs::create_dir(dir).map_err(|e| format!("{}：{e}", dir.display()))?;
    let files = std::iter::once((COMPOSE_FILE, package.source.as_bytes())).chain(
        package
            .files()
            .iter()
            .map(|(path, data)| (path.as_str(), data.as_slice())),
    );
    for (path, data) in files {
        let path = dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}：{e}", parent.display()))?;
        }
        std::fs::write(&path, data).map_err(|e| format!("{}：{e}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
