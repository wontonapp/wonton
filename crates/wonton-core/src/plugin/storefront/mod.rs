//! 商店（ADR-0017）：官方索引和 GitHub 上的社区插件，各自缓存，取不到时给上次的结果并如实说明。
//!
//! - 官方索引：设了 `WONTON_STORE_INDEX` 就每次读那个本地目录；否则从 GitHub 取 `geraldpeng6/wonton-store` 的
//!   `index.json`，6 小时一次，raw 不通时走 API。取到的存一份到磁盘，核心重启后取不到时也有列表。
//! - 社区插件：搜 topic `wonton-plugin` 的公开仓库，10 分钟一次；主人可以关掉（`WONTON_STORE_COMMUNITY=off`）。
//! - App 要求刷新（`?refresh=true`）时不等缓存过期，但离上次去取不到一分钟就不再取：GitHub 的搜索 API 不带令牌时
//!   每分钟只给 10 次。

pub mod added;
mod community;
pub mod index;
mod listing;
pub mod onboarding;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use wonton_protocol::{StoreFailure, StoreProblem};

use super::github::{GitHub, GitHubError, Repo};
use super::manager::now_secs;
use super::store::{create_private_dir, read_json, write_json};
use crate::plugin::languages::Languages;
use added::AddedSources;
pub use community::{Community, Listing};
use index::{Files, Index, IndexError};

/// 官方索引所在的仓库。
const STORE_REPO: (&str, &str) = ("geraldpeng6", "wonton-store");
pub(in crate::plugin) const OFFICIAL_TTL: Duration = Duration::from_secs(6 * 3600);
pub(in crate::plugin) const COMMUNITY_TTL: Duration = Duration::from_secs(10 * 60);
/// 两次去来源取之间至少隔这么久，App 要求刷新也一样。
pub(in crate::plugin) const MIN_RETRY: Duration = Duration::from_secs(60);
const INDEX_FILE: &str = "index.json";
/// `index.json` 最多这么大。
const MAX_INDEX_BYTES: u64 = 8 << 20;

/// 商店的配置。
#[derive(Debug, Clone)]
pub struct StoreConfig {
    /// 本地的索引目录（`WONTON_STORE_INDEX`）：`index.json` 和 `blobs/<sha256>`。没设时从 GitHub 取官方索引。
    pub index_dir: Option<PathBuf>,
    /// 列出社区插件（`WONTON_STORE_COMMUNITY` 不是 `off`）。
    pub community: bool,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            index_dir: None,
            community: true,
        }
    }
}

/// 商店的一部分：上次取到的结果，什么时候取到的，最近一次去取的时间和问题。
struct Section<T> {
    value: Option<T>,
    fetched_at_ms: Option<u64>,
    fetched: Option<Instant>,
    attempted: Option<Instant>,
    problem: Option<StoreProblem>,
}

impl<T> Default for Section<T> {
    fn default() -> Self {
        Self {
            value: None,
            fetched_at_ms: None,
            fetched: None,
            attempted: None,
            problem: None,
        }
    }
}

impl<T> Section<T> {
    /// 该不该去来源取：从没取到过、过期了、或者 App 要求刷新；离上次去取不到一分钟时都不取。
    fn due(&self, ttl: Duration, refresh: bool) -> bool {
        if self.attempted.is_some_and(|t| t.elapsed() < MIN_RETRY) {
            return false;
        }
        refresh || self.fetched.is_none_or(|t| t.elapsed() >= ttl)
    }

    fn succeeded(&mut self, value: T, problem: Option<StoreProblem>) {
        self.value = Some(value);
        self.fetched_at_ms = Some(now_secs() * 1000);
        self.fetched = Some(Instant::now());
        self.attempted = self.fetched;
        self.problem = problem;
    }

    /// 没取到：留着上次的结果和它的时间。
    fn failed(&mut self, reason: StoreFailure, details: String) {
        self.attempted = Some(Instant::now());
        self.problem = Some(StoreProblem { reason, details });
    }
}

/// 磁盘上的官方索引。索引留原文：下次还按当时的核心的规则解析。
#[derive(Serialize, Deserialize)]
struct CachedIndex {
    fetched_at_ms: u64,
    index: serde_json::Value,
}

pub struct Storefront {
    github: GitHub,
    config: StoreConfig,
    /// 磁盘上的官方索引的路径。
    cache: PathBuf,
    official: tokio::sync::Mutex<Section<Index>>,
    community: tokio::sync::Mutex<Section<Listing>>,
    /// 读过的社区仓库：仓库 → （推送时间，读到的结果）。仓库没有新推送就不重读。
    read: tokio::sync::Mutex<community::Read>,
    /// 主人添加的来源。在后台读，所以放在 `Arc` 里。
    pub added: Arc<AddedSources>,
}

impl Storefront {
    pub fn new(github: GitHub, config: StoreConfig, cache: PathBuf) -> Self {
        let added = AddedSources::load(github.clone(), cache.with_file_name("sources.json"));
        Self {
            added: Arc::new(added),
            github,
            config,
            cache,
            official: tokio::sync::Mutex::default(),
            community: tokio::sync::Mutex::default(),
            read: tokio::sync::Mutex::default(),
        }
    }

    pub fn config(&self) -> &StoreConfig {
        &self.config
    }

    /// 官方索引的文件从哪里取。
    pub fn files(&self) -> Files {
        match &self.config.index_dir {
            Some(dir) => Files::Dir(dir.clone()),
            None => Files::GitHub(self.github.clone()),
        }
    }

    /// 官方索引，和它的时间、问题。本地目录每次都重读：读一次只要几毫秒，目录会随镜像或测试变化。
    pub async fn official(
        &self,
        refresh: bool,
        langs: &Languages,
    ) -> (Index, Option<u64>, Option<StoreProblem>) {
        let mut section = self.official.lock().await;
        if let Some(dir) = &self.config.index_dir {
            let path = dir.join("index.json");
            let read = std::fs::read(&path)
                .map_err(|e| {
                    let details = crate::l10n::text(
                        langs,
                        "plugin-settings-store-index-read-failed",
                        &[
                            ("path", path.display().to_string().into()),
                            ("error", e.to_string().into()),
                        ],
                    );
                    (StoreFailure::Upstream, details)
                })
                .and_then(|json| Index::parse(&json, langs).map_err(|e| index_failure(&e, langs)));
            return match read {
                Ok(index) => (index, None, None),
                Err((reason, details)) => (
                    Index::default(),
                    None,
                    Some(StoreProblem { reason, details }),
                ),
            };
        }
        if section.attempted.is_none() {
            restore(&self.cache, &mut section);
        }
        if section.due(OFFICIAL_TTL, refresh) {
            match self.fetch_index(langs).await {
                Ok((index, raw)) => {
                    section.succeeded(index, None);
                    save(&self.cache, section.fetched_at_ms, raw);
                }
                Err((reason, details)) => section.failed(reason, details),
            }
        }
        (
            section.value.clone().unwrap_or_default(),
            section.fetched_at_ms,
            section.problem.clone(),
        )
    }

    async fn fetch_index(
        &self,
        langs: &Languages,
    ) -> Result<(Index, serde_json::Value), (StoreFailure, String)> {
        let repo = Repo {
            owner: STORE_REPO.0.to_owned(),
            name: STORE_REPO.1.to_owned(),
        };
        let json = match self
            .github
            .file(&repo, "main", INDEX_FILE, MAX_INDEX_BYTES, langs)
            .await
        {
            Ok(json) => json,
            Err(raw @ (GitHubError::Unavailable(_) | GitHubError::Timeout(_))) => {
                tracing::info!(error = %raw, "raw 取不到官方索引，改走 API");
                self.github
                    .file_via_api(&repo, "main", INDEX_FILE, MAX_INDEX_BYTES, langs)
                    .await
                    .map_err(|api| {
                        (
                            failure(&api),
                            format!(
                                "{}；{}",
                                crate::l10n::text(
                                    langs,
                                    "plugin-settings-store-index-fetch-failed",
                                    &[("error", raw.to_string().into())],
                                ),
                                api
                            ),
                        )
                    })?
            }
            Err(e) => {
                return Err((
                    failure(&e),
                    crate::l10n::text(
                        langs,
                        "plugin-settings-store-index-fetch-failed",
                        &[("error", e.to_string().into())],
                    ),
                ));
            }
        };
        let index = Index::parse(&json, langs).map_err(|e| index_failure(&e, langs))?;
        let raw = serde_json::from_slice(&json).map_err(|e| {
            let error = IndexError::Invalid(e.to_string());
            index_failure(&error, langs)
        })?;
        Ok((index, raw))
    }

    /// 社区插件，和它的时间、问题。`official` 里的仓库和插件 ID 不列出。
    pub async fn community(
        &self,
        official: &Index,
        refresh: bool,
        langs: &Languages,
    ) -> (Listing, Option<u64>, Option<StoreProblem>) {
        if !self.config.community {
            let problem = StoreProblem {
                reason: StoreFailure::Disabled,
                details: "WONTON_STORE_COMMUNITY=off".to_owned(),
            };
            return (Listing::default(), None, Some(problem));
        }
        let mut section = self.community.lock().await;
        if section.due(COMMUNITY_TTL, refresh) {
            let mut read = self.read.lock().await;
            match community::discover(&self.github, official, &mut read, langs).await {
                Ok((found, problem)) => section.succeeded(found, problem),
                Err(e) => section.failed(
                    failure(&e),
                    crate::l10n::text(
                        langs,
                        "plugin-settings-store-community-unavailable",
                        &[("error", e.to_string().into())],
                    ),
                ),
            }
        }
        let mut listing = section.value.clone().unwrap_or_default();
        listing.plugins.retain(|c| {
            !official
                .plugins
                .iter()
                .any(|o| o.repo == c.repo || o.manifest.id == c.manifest.id)
        });
        (listing, section.fetched_at_ms, section.problem.clone())
    }
}

#[cfg(test)]
impl Storefront {
    /// 把官方索引上次取和上次去取的时间往前挪：测试不用真等 [`MIN_RETRY`]。
    pub(in crate::plugin) async fn age_official(&self, by: Duration) {
        let mut section = self.official.lock().await;
        let back = |t: Instant| t.checked_sub(by).expect("测试挪的时间太长");
        section.fetched = section.fetched.map(back);
        section.attempted = section.attempted.map(back);
    }
}

impl Storefront {
    /// 社区插件列表里这个仓库的星数；没列出过时为空。
    pub async fn stars(&self, repo: &Repo) -> Option<u32> {
        let section = self.community.lock().await;
        let plugins = &section.value.as_ref()?.plugins;
        let found = plugins.iter().find(|c| &c.repo == repo)?;
        Some(found.stars)
    }
}

/// 读磁盘上的官方索引。时间照旧，所以 `due` 仍然会去取一次；读不了的当作没有。
fn restore(path: &Path, section: &mut Section<Index>) {
    let langs = crate::plugin::languages::Languages::default();
    let cached: CachedIndex = match read_json(path) {
        Ok(Some(cached)) => cached,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%error, "磁盘上的官方索引读不了，当作没有");
            return;
        }
    };
    let index = serde_json::to_vec(&cached.index)
        .map_err(|e| e.to_string())
        .and_then(|json| Index::parse(&json, &langs).map_err(|e| e.to_string()));
    match index {
        Ok(index) => {
            section.value = Some(index);
            section.fetched_at_ms = Some(cached.fetched_at_ms);
        }
        Err(error) => tracing::warn!(%error, "磁盘上的官方索引解析不了，当作没有"),
    }
}

/// 存官方索引。存不下只影响重启后取不到来源的时候，这次的列表照常给。
fn save(path: &Path, fetched_at_ms: Option<u64>, index: serde_json::Value) {
    let cached = CachedIndex {
        fetched_at_ms: fetched_at_ms.unwrap_or_default(),
        index,
    };
    let written = path
        .parent()
        .map_or(Ok(()), create_private_dir)
        .and_then(|()| write_json(path, &cached));
    if let Err(error) = written {
        tracing::error!(%error, "官方索引存不到磁盘");
    }
}

fn index_failure(e: &IndexError, langs: &Languages) -> (StoreFailure, String) {
    let reason = match e {
        IndexError::NewerFormat(_) => StoreFailure::IndexTooNew,
        IndexError::Invalid(_) => StoreFailure::InvalidIndex,
    };
    (reason, e.message(langs))
}

pub(in crate::plugin) fn failure(e: &GitHubError) -> StoreFailure {
    match e {
        GitHubError::RateLimited(_) => StoreFailure::RateLimited,
        GitHubError::Unavailable(_) => StoreFailure::Unreachable,
        GitHubError::Timeout(_) => StoreFailure::Timeout,
        GitHubError::NotFound(_) | GitHubError::TooBig(_) => StoreFailure::Upstream,
    }
}
