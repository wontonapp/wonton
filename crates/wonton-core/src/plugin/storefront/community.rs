//! 社区插件的发现（ADR-0017 的“社区插件”）：搜 topic `wonton-plugin`、没归档、不是 fork 的公开仓库，按星数排，最多
//! 1000 个；官方索引里的仓库不读，其余的取最新正式版本的 compose 和图标。没有正式版本的不列出，日志里记下原因；清单读不懂的
//! 告诉主人。

use std::collections::HashMap;

use futures_util::StreamExt;
use wonton_protocol::{StoreFailure, StoreProblem, StoreUnreadable};

use super::super::gallery::IconSource;
use super::super::github::{Found, GitHub, GitHubError, Repo, Tag};
use super::super::github_source::latest;
use super::super::languages::Languages;
use super::super::manifest::Manifest;
use super::super::package::PluginPackage;
use super::super::package_files::{COMPOSE_FILE, MAX_FILE_BYTES, normalize};
use super::index::Index;

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

/// 搜一轮的结果。
#[derive(Debug, Clone, Default)]
pub struct Listing {
    pub plugins: Vec<Community>,
    pub unreadable: Vec<StoreUnreadable>,
}

/// 读一个仓库的结果。
#[derive(Debug, Clone)]
pub enum RepoRead {
    Plugin(Box<Community>),
    /// 不是给商店的正式版本（没有版本 tag、没有 compose 文件之类），日志里有原因。
    NotListed,
    Unreadable(String),
}

/// 读过的仓库：仓库 → （推送时间，读到的结果）。
pub type Read = HashMap<Repo, (String, RepoRead)>;

/// 搜一轮，读有新推送的仓库。搜索的第一页就失败时报错；后面的页、个别仓库没读到时列表不完整，如实说明。
pub async fn discover(
    github: &GitHub,
    official: &Index,
    read: &mut Read,
    langs: &Languages,
) -> Result<(Listing, Option<StoreProblem>), GitHubError> {
    let mut found: Vec<Found> = Vec::new();
    let mut missing = Vec::new();
    for page in 1..=MAX_PAGES {
        match github.search(QUERY, page, langs).await {
            Ok(result) => {
                let full = result.repos.len() == 100;
                found.extend(result.repos);
                if !full || found.len() >= result.total as usize {
                    break;
                }
            }
            Err(e) if page == 1 => return Err(e),
            Err(e) => {
                missing.push(crate::l10n::text(
                    langs,
                    "plugin-settings-store-community-page-failed",
                    &[
                        ("page", page.to_string().into()),
                        ("error", e.to_string().into()),
                    ],
                ));
                break;
            }
        }
    }
    let jobs: Vec<(Found, Option<RepoRead>)> = found
        .into_iter()
        .filter(|f| !official.plugins.iter().any(|o| o.repo == f.repo))
        .filter(|f| {
            let source = format!("github:{}", f.repo);
            !official.skipped.iter().any(|s| s.source == source)
        })
        .map(|f| {
            let cached = read
                .get(&f.repo)
                .filter(|(pushed_at, _)| *pushed_at == f.pushed_at)
                .map(|(_, plugin)| plugin.clone());
            (f, cached)
        })
        .collect();
    let results: Vec<(Found, Result<RepoRead, GitHubError>)> = futures_util::stream::iter(jobs)
        .map(|(f, cached)| async move {
            let result = match cached {
                Some(plugin) => Ok(plugin),
                None => read_repo(github, &f, langs).await,
            };
            (f, result)
        })
        .buffered(PARALLEL)
        .collect()
        .await;
    let mut listing = Listing::default();
    for (f, result) in results {
        match result {
            Ok(repo) => {
                read.insert(f.repo.clone(), (f.pushed_at.clone(), repo.clone()));
                match repo {
                    RepoRead::Plugin(plugin) => listing.plugins.push(Community {
                        stars: f.stars,
                        ..*plugin
                    }),
                    RepoRead::NotListed => {}
                    RepoRead::Unreadable(reason) => listing.unreadable.push(StoreUnreadable {
                        source: format!("github:{}", f.repo),
                        reason,
                    }),
                }
            }
            Err(e) => missing.push(format!("{}（{e}）", f.repo)),
        }
    }
    let problem = (!missing.is_empty()).then(|| StoreProblem {
        reason: StoreFailure::Incomplete,
        details: crate::l10n::text(
            langs,
            "plugin-settings-store-community-partial",
            &[("missing", crate::l10n::list(langs, &missing).into())],
        ),
    });
    Ok((listing, problem))
}

/// 读一个仓库最新正式版本的清单和图标。连不上 GitHub 之类的返回错误，下次再读。
async fn read_repo(
    github: &GitHub,
    found: &Found,
    langs: &Languages,
) -> Result<RepoRead, GitHubError> {
    let repo = &found.repo;
    let skip = |why: String| {
        tracing::info!(%repo, reason = %why, "社区插件不列出");
        Ok(RepoRead::NotListed)
    };
    let unreadable = |why: String| {
        tracing::warn!(%repo, reason = %why, "社区插件读不懂，不列出");
        Ok(RepoRead::Unreadable(why))
    };
    let tags = match github.tags(repo, langs).await {
        Err(GitHubError::NotFound(why)) => return skip(why),
        other => other?,
    };
    let Some(tag) = latest(tags) else {
        return skip(crate::l10n::text(
            langs,
            "plugin-settings-github-no-release-tag",
            &[],
        ));
    };
    let compose = match github
        .file(repo, &tag.commit, COMPOSE_FILE, MAX_FILE_BYTES, langs)
        .await
    {
        Err(e @ (GitHubError::NotFound(_) | GitHubError::TooBig(_))) => return skip(e.to_string()),
        other => other?,
    };
    let Ok(compose) = String::from_utf8(compose) else {
        return unreadable(crate::l10n::text(
            langs,
            "plugin-settings-upgrade-compose-not-utf8",
            &[("file", COMPOSE_FILE.into())],
        ));
    };
    let manifest = match PluginPackage::manifest_of(&compose, langs) {
        Ok(manifest) => manifest,
        Err(e) => return unreadable(format!("{e}")),
    };
    if tag.name.strip_prefix('v') != Some(manifest.version.as_str()) {
        return skip(crate::l10n::text(
            langs,
            "plugin-settings-github-tag-version-short",
            &[
                ("tag", tag.name.as_str().into()),
                ("version", manifest.version.as_str().into()),
            ],
        ));
    }
    let icon = match manifest.icon.as_deref().map(normalize) {
        Some(Ok(path)) => match github
            .file(repo, &tag.commit, &path, MAX_FILE_BYTES, langs)
            .await
        {
            Ok(data) => IconSource::from_file(&path, data),
            Err(e @ (GitHubError::NotFound(_) | GitHubError::TooBig(_))) => {
                tracing::info!(%repo, error = %e, "社区插件的图标取不到，列出时没有图标");
                None
            }
            Err(e) => return Err(e),
        },
        Some(Err(_)) | None => None,
    };
    Ok(RepoRead::Plugin(Box::new(Community {
        repo: repo.clone(),
        stars: found.stars,
        tag,
        manifest,
        icon,
    })))
}
