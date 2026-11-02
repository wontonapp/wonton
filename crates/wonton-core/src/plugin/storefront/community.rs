//! 社区插件的发现（ADR-0017 的“社区插件”）：搜 topic `wonton-plugin`、没归档、不是 fork 的公开仓库，按星数排，最多
//! 1000 个；每个仓库取最新正式版本的 compose 和图标。没有正式版本、清单读不懂的不列出，日志里记下原因。

use std::collections::HashMap;

use futures_util::StreamExt;
use wonton_protocol::{StoreFailure, StoreProblem};

use super::super::gallery::IconSource;
use super::super::github::{Found, GitHub, GitHubError, Repo, Tag};
use super::super::github_source::latest;
use super::super::manifest::Manifest;
use super::super::package::PluginPackage;
use super::super::package_files::{COMPOSE_FILE, MAX_FILE_BYTES, normalize};

const QUERY: &str = "topic:wonton-plugin archived:false fork:false";
/// 每页 100 个，GitHub 最多给前 1000 个。
const MAX_PAGES: u32 = 10;
/// 同时读这么多个仓库。
const PARALLEL: usize = 8;

/// 商店里的一个社区插件。
#[derive(Debug, Clone)]
pub struct Community {
    pub repo: Repo,
    pub stars: u32,
    pub tag: Tag,
    pub manifest: Manifest,
    pub icon: Option<IconSource>,
}

/// 读过的仓库：仓库 → （推送时间，读到的插件；不列出时为空）。
pub type Read = HashMap<Repo, (String, Option<Community>)>;

/// 搜一轮，读有新推送的仓库。搜索的第一页就失败时报错；后面的页、个别仓库没读到时列表不完整，如实说明。
pub async fn discover(
    github: &GitHub,
    read: &mut Read,
) -> Result<(Vec<Community>, Option<StoreProblem>), GitHubError> {
    let mut found: Vec<Found> = Vec::new();
    let mut missing = Vec::new();
    for page in 1..=MAX_PAGES {
        match github.search(QUERY, page).await {
            Ok(result) => {
                let full = result.repos.len() == 100;
                found.extend(result.repos);
                if !full || found.len() >= result.total as usize {
                    break;
                }
            }
            Err(e) if page == 1 => return Err(e),
            Err(e) => {
                missing.push(format!("搜索结果的第 {page} 页（{e}）"));
                break;
            }
        }
    }
    let jobs: Vec<(Found, Option<Option<Community>>)> = found
        .into_iter()
        .map(|f| {
            let cached = read
                .get(&f.repo)
                .filter(|(pushed_at, _)| *pushed_at == f.pushed_at)
                .map(|(_, plugin)| plugin.clone());
            (f, cached)
        })
        .collect();
    let results: Vec<(Found, Result<Option<Community>, GitHubError>)> =
        futures_util::stream::iter(jobs)
            .map(|(f, cached)| async move {
                let result = match cached {
                    Some(plugin) => Ok(plugin),
                    None => read_repo(github, &f).await,
                };
                (f, result)
            })
            .buffered(PARALLEL)
            .collect()
            .await;
    let mut listed = Vec::new();
    for (f, result) in results {
        match result {
            Ok(plugin) => {
                read.insert(f.repo.clone(), (f.pushed_at.clone(), plugin.clone()));
                listed.extend(plugin.map(|p| Community {
                    stars: f.stars,
                    ..p
                }));
            }
            Err(e) => missing.push(format!("{}（{e}）", f.repo)),
        }
    }
    let problem = (!missing.is_empty()).then(|| StoreProblem {
        reason: StoreFailure::Incomplete,
        message: format!("有的没读到，列表不完整：{}", missing.join("、")),
    });
    Ok((listed, problem))
}

/// 读一个仓库最新正式版本的清单和图标。仓库本身的问题（没有正式版本、清单读不懂）返回 `Ok(None)`，不列出；
/// 连不上 GitHub 之类的返回错误，下次再读。
async fn read_repo(github: &GitHub, found: &Found) -> Result<Option<Community>, GitHubError> {
    let repo = &found.repo;
    let skip = |why: String| {
        tracing::info!(%repo, reason = %why, "社区插件不列出");
        Ok(None)
    };
    let tags = match github.tags(repo).await {
        Err(GitHubError::NotFound(why)) => return skip(why),
        other => other?,
    };
    let Some(tag) = latest(tags) else {
        return skip("没有 v<语义化版本> 的正式版本 tag".to_owned());
    };
    let compose = match github
        .file(repo, &tag.commit, COMPOSE_FILE, MAX_FILE_BYTES)
        .await
    {
        Err(e @ (GitHubError::NotFound(_) | GitHubError::TooBig(_))) => return skip(e.to_string()),
        other => other?,
    };
    let Ok(compose) = String::from_utf8(compose) else {
        return skip(format!("{COMPOSE_FILE} 不是 UTF-8 文本"));
    };
    let manifest = match PluginPackage::manifest_of(&compose) {
        Ok(manifest) => manifest,
        Err(e) => return skip(format!("清单读不懂：{e}")),
    };
    if tag.name.strip_prefix('v') != Some(manifest.version.as_str()) {
        return skip(format!(
            "tag {} 和清单的版本 {} 不一致",
            tag.name, manifest.version
        ));
    }
    let icon = match manifest.icon.as_deref().map(normalize) {
        Some(Ok(path)) => match github.file(repo, &tag.commit, &path, MAX_FILE_BYTES).await {
            Ok(data) => IconSource::from_file(&path, data),
            Err(e @ (GitHubError::NotFound(_) | GitHubError::TooBig(_))) => {
                tracing::info!(%repo, error = %e, "社区插件的图标取不到，列出时没有图标");
                None
            }
            Err(e) => return Err(e),
        },
        Some(Err(_)) | None => None,
    };
    Ok(Some(Community {
        repo: repo.clone(),
        stars: found.stars,
        tag,
        manifest,
        icon,
    }))
}
