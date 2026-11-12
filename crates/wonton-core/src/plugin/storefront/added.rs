//! 主人添加的来源（ADR-0031）：一个插件仓库，或者一个第三方索引。地址存在 `sources.json`，读到的内容只放内存。每个来源
//! 有自己的缓存和问题，一个坏了只影响它那一段；列表不等慢的来源。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use wonton_protocol::{SourceKind, StoreFailure, StoreProblem, StoreUnreadable};

use super::super::gallery::IconSource;
use super::super::github::{GitHub, GitHubError, Repo};
use super::super::github_source::{self, SourceRef, Wanted, github_error};
use super::super::index_location::IndexLocation;
use super::super::install_error::InstallError;
use super::super::manager::now_secs;
use super::super::manifest::Manifest;
use super::super::package::PluginPackage;
use super::super::package_files::{COMPOSE_FILE, normalize};
use super::super::source_address::Address;
use super::super::store::{create_private_dir, read_json, write_json};
use super::index::{Files, Index, Listed};
use super::{COMMUNITY_TTL, MAX_INDEX_BYTES, OFFICIAL_TTL, Section};

const MAX_SOURCES: usize = 50;
/// 列表最多等读得快的来源这么久，读得慢的接着在后台读。
const LIST_WAIT: Duration = Duration::from_secs(3);
/// 同时读索引里这么多个插件的图标。
const PARALLEL_ICONS: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Saved {
    address: String,
    kind: SavedKind,
    added_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SavedKind {
    Repo,
    Index,
}

#[derive(Default, Serialize, Deserialize)]
struct SourcesFile {
    sources: Vec<Saved>,
}

/// 读到的内容。
#[derive(Clone)]
enum Contents {
    Repo {
        manifest: Box<Manifest>,
        icon: Option<IconSource>,
    },
    Index {
        index: Box<Index>,
        icons: HashMap<String, IconSource>,
    },
}

struct Source {
    saved: Saved,
    section: Section<Contents>,
    reading: bool,
}

/// 商店里的一个来源，给 App 的视图用。
pub struct AddedView {
    pub address: String,
    pub kind: SourceKind,
    pub provider: String,
    pub name: Option<String>,
    pub repo: Option<Repo>,
    pub plugins: Vec<AddedPlugin>,
    pub fetched_at_ms: Option<u64>,
    pub problem: Option<StoreProblem>,
    pub unreadable: Vec<StoreUnreadable>,
}

pub struct AddedPlugin {
    pub source: String,
    pub provider: String,
    pub via_index: Option<String>,
    pub manifest: Manifest,
    pub icon: Option<IconSource>,
}

pub struct AddedSources {
    github: GitHub,
    path: PathBuf,
    sources: Mutex<Vec<Source>>,
    /// `sources.json` 读不了的原因。这时不能写，否则主人添加过的就都没了。
    unreadable: Option<String>,
}

/// 主人填的地址说的是什么。
enum Target {
    Repo(SourceRef),
    Index(IndexLocation),
}

impl Target {
    fn of(saved: &Saved) -> Result<Self, InstallError> {
        match saved.kind {
            SavedKind::Repo => SourceRef::parse(&saved.address).map(Self::Repo),
            SavedKind::Index => IndexLocation::parse(&saved.address).map(Self::Index),
        }
    }

    fn address(&self) -> String {
        match self {
            Self::Repo(source) => source.text(),
            Self::Index(location) => location.address(),
        }
    }

    fn kind(&self) -> SavedKind {
        match self {
            Self::Repo(_) => SavedKind::Repo,
            Self::Index(_) => SavedKind::Index,
        }
    }
}

impl AddedSources {
    pub fn load(github: GitHub, path: PathBuf) -> Self {
        let (saved, unreadable) = match read_json::<SourcesFile>(&path) {
            Ok(file) => (file.unwrap_or_default().sources, None),
            Err(error) => {
                tracing::error!(%error, "添加的来源读不了");
                (Vec::new(), Some(error.to_string()))
            }
        };
        let sources = saved
            .into_iter()
            .map(|saved| Source {
                saved,
                section: Section::default(),
                reading: false,
            })
            .collect();
        Self {
            github,
            path,
            sources: Mutex::new(sources),
            unreadable,
        }
    }

    fn sources(&self) -> MutexGuard<'_, Vec<Source>> {
        self.sources.lock().expect("添加的来源锁中毒")
    }

    /// 添加：能读到才写下来。`official` 是官方索引：里面的仓库不带版本时不用添加。
    pub async fn add(&self, address: &str, official: &Index) -> Result<AddedView, InstallError> {
        if let Some(why) = &self.unreadable {
            return Err(InstallError::Storage(format!("添加的来源读不了：{why}")));
        }
        let target = self.target(Address::parse(address)?, official).await?;
        let address = target.address();
        self.check_room(&address)?;
        let contents = read(&self.github, &target).await?;
        let added = {
            let mut sources = self.sources();
            if sources.iter().any(|s| s.saved.address == address) {
                return Err(already_added(&address));
            }
            let saved = Saved {
                address,
                kind: target.kind(),
                added_at_ms: now_secs() * 1000,
            };
            let mut section = Section::default();
            section.succeeded(contents, None);
            sources.push(Source {
                saved,
                section,
                reading: false,
            });
            if let Err(e) = self.save(&sources) {
                sources.pop();
                return Err(e);
            }
            view(sources.last().expect("刚加进去"))
        };
        Ok(added)
    }

    /// 删除只影响发现：记录里的 `origin` 自带来处，已安装的插件照常运行。
    pub fn remove(&self, address: &str) -> Result<(), InstallError> {
        let mut sources = self.sources();
        let Some(at) = sources.iter().position(|s| s.saved.address == address) else {
            return Err(InstallError::NotFound(format!("没有添加过 {address}")));
        };
        let removed = sources.remove(at);
        if let Err(e) = self.save(&sources) {
            sources.insert(at, removed);
            return Err(e);
        }
        Ok(())
    }

    fn check_room(&self, address: &str) -> Result<(), InstallError> {
        let sources = self.sources();
        if sources.iter().any(|s| s.saved.address == address) {
            return Err(already_added(address));
        }
        if sources.len() >= MAX_SOURCES {
            return Err(InstallError::Invalid(format!(
                "最多添加 {MAX_SOURCES} 个来源，先删掉不用的"
            )));
        }
        Ok(())
    }

    fn save(&self, sources: &[Source]) -> Result<(), InstallError> {
        let file = SourcesFile {
            sources: sources.iter().map(|s| s.saved.clone()).collect(),
        };
        self.path
            .parent()
            .map_or(Ok(()), create_private_dir)
            .and_then(|()| write_json(&self.path, &file))
            .map_err(|e| InstallError::Storage(format!("添加的来源存不了：{e}")))
    }

    /// 地址 → 插件仓库或索引。仓库不带版本时看默认分支的根目录：有 compose 是插件，有 `index.json` 是索引。
    async fn target(&self, address: Address, official: &Index) -> Result<Target, InstallError> {
        let source = match address {
            Address::Url(url) => return IndexLocation::parse(url.as_str()).map(Target::Index),
            Address::Repo(source) => source,
        };
        if source.wanted != Wanted::Latest {
            return Ok(Target::Repo(source));
        }
        if official.find(&source).is_some() {
            return Err(InstallError::Conflict(format!(
                "{} 已经在官方商店里了",
                source.repo
            )));
        }
        let at_head = |path: &'static str| {
            let github = self.github.clone();
            let repo = source.repo.clone();
            async move { github.file(&repo, "HEAD", path, MAX_INDEX_BYTES).await }
        };
        match at_head(COMPOSE_FILE).await {
            Ok(_) => return Ok(Target::Repo(source)),
            Err(GitHubError::NotFound(_)) => {}
            Err(e) => return Err(github_error(e)),
        }
        match at_head(super::INDEX_FILE).await {
            Ok(_) => Ok(Target::Index(IndexLocation::Repo(source.repo))),
            Err(GitHubError::NotFound(_)) => Err(InstallError::Invalid(format!(
                "{} 的根目录没有 {COMPOSE_FILE}，也没有 {}：不是插件，也不是索引",
                source.repo,
                super::INDEX_FILE
            ))),
            Err(e) => Err(github_error(e)),
        }
    }

    /// 读该读的来源，最多等 [`LIST_WAIT`]；没读完的接着在后台读。
    pub async fn refresh(self: &Arc<Self>, refresh: bool) {
        let due: Vec<Saved> = self
            .sources()
            .iter_mut()
            .filter(|s| {
                let ttl = match s.saved.kind {
                    SavedKind::Repo => COMMUNITY_TTL,
                    SavedKind::Index => OFFICIAL_TTL,
                };
                !s.reading && s.section.due(ttl, refresh)
            })
            .map(|s| {
                s.reading = true;
                s.saved.clone()
            })
            .collect();
        let reads = due.into_iter().map(|saved| {
            let this = Arc::clone(self);
            tokio::spawn(async move { this.read_one(saved).await })
        });
        let all = futures_util::future::join_all(reads);
        if tokio::time::timeout(LIST_WAIT, all).await.is_err() {
            tracing::debug!("有的来源还没读完，先给上次的结果");
        }
    }

    async fn read_one(&self, saved: Saved) {
        let result = match Target::of(&saved) {
            Ok(target) => read(&self.github, &target).await,
            Err(e) => Err(e),
        };
        let mut sources = self.sources();
        let Some(source) = sources
            .iter_mut()
            .find(|s| s.saved.address == saved.address)
        else {
            return;
        };
        source.reading = false;
        match result {
            Ok(contents) => source.section.succeeded(contents, None),
            Err(e) => {
                tracing::warn!(address = %saved.address, error = %e, "添加的来源没读到");
                source.section.failed(store_failure(&e), e.to_string());
            }
        }
    }

    pub fn views(&self) -> Vec<AddedView> {
        self.sources().iter().map(view).collect()
    }

    /// 这个索引还在“我添加的”里。
    pub fn has_index(&self, location: &IndexLocation) -> bool {
        let address = location.address();
        self.sources().iter().any(|s| s.saved.address == address)
    }

    /// 索引里这个插件的最新一项。
    pub async fn listed_in(
        &self,
        location: &IndexLocation,
        id: &str,
    ) -> Result<Listed, InstallError> {
        let index = read_index(&self.github, location).await?;
        index
            .plugins
            .into_iter()
            .find(|p| p.manifest.id.to_string() == id)
            .ok_or_else(|| {
                InstallError::NotFound(format!("索引 {} 里没有 {id}", location.provider()))
            })
    }

    /// 索引里这个插件的插件包，文件按索引核对。信任级别和官方的不同，来源写的是这个索引。
    pub async fn package_in(
        &self,
        location: &IndexLocation,
        id: &str,
    ) -> Result<PluginPackage, InstallError> {
        let listed = self.listed_in(location, id).await?;
        let files = Files::Third {
            location: location.clone(),
            github: self.github.clone(),
        };
        let mut package = files.package(&listed).await?;
        let mut origin = listed.origin();
        origin.trust = github_source::Trust::Community;
        origin.index = Some(location.item_source(id));
        package.origin = Some(origin);
        Ok(package)
    }
}

fn already_added(address: &str) -> InstallError {
    InstallError::Conflict(format!("{address} 已经添加过了"))
}

/// 给 App 看的原因，详细的写在 `details` 里。
fn store_failure(e: &InstallError) -> StoreFailure {
    match e {
        InstallError::SourceUnavailable(_) => StoreFailure::Unreachable,
        InstallError::Invalid(_) => StoreFailure::InvalidIndex,
        _ => StoreFailure::Upstream,
    }
}

async fn read(github: &GitHub, target: &Target) -> Result<Contents, InstallError> {
    match target {
        Target::Repo(source) => {
            let origin = github_source::resolve(github, source).await?;
            let package = github_source::fetch(github, &origin).await?;
            Ok(Contents::Repo {
                icon: IconSource::of(&package),
                manifest: Box::new(package.manifest),
            })
        }
        Target::Index(location) => {
            let index = read_index(github, location).await?;
            let icons = read_icons(github, location, &index).await;
            Ok(Contents::Index {
                index: Box::new(index),
                icons,
            })
        }
    }
}

async fn read_index(github: &GitHub, location: &IndexLocation) -> Result<Index, InstallError> {
    let json = location
        .read_index(github, MAX_INDEX_BYTES)
        .await
        .map_err(|e| match e {
            // 仓库和地址都没错，只是没有索引：和连不上分开。
            GitHubError::NotFound(_) => {
                InstallError::NotFound(format!("{} 没有索引文件", location.provider()))
            }
            other => github_error(other),
        })?;
    Index::parse(&json)
        .map_err(|e| InstallError::Invalid(format!("{} 不是索引：{e}", location.address())))
}

/// 列表上的图标。取不到时没有图标，不影响列表。
async fn read_icons(
    github: &GitHub,
    location: &IndexLocation,
    index: &Index,
) -> HashMap<String, IconSource> {
    let files = Files::Third {
        location: location.clone(),
        github: github.clone(),
    };
    let mut icons = HashMap::new();
    // 要交给 `tokio::spawn`，这里的 future 不能借用 `index`：借用的闭包过不了 `Send` 的高阶生命周期检查。
    let mut reads = futures_util::stream::iter(index.plugins.clone())
        .map(move |listed| {
            let files = files.clone();
            async move { read_icon(&files, &listed).await }
        })
        .buffer_unordered(PARALLEL_ICONS);
    while let Some(read) = reads.next().await {
        icons.extend(read);
    }
    icons
}

async fn read_icon(files: &Files, listed: &Listed) -> Option<(String, IconSource)> {
    let path = normalize(listed.manifest.icon.as_deref()?).ok()?;
    match files.file(listed, &path).await {
        Ok(data) => {
            let icon = IconSource::from_file(&path, data)?;
            Some((listed.manifest.id.to_string(), icon))
        }
        Err(e) => {
            tracing::info!(source = %listed.entry.source, error = %e, "索引里的图标取不到");
            None
        }
    }
}

fn view(source: &Source) -> AddedView {
    let saved = &source.saved;
    let target = Target::of(saved).ok();
    let provider = match &target {
        Some(Target::Repo(s)) => format!("github.com/{}", s.repo),
        Some(Target::Index(location)) => location.provider(),
        None => saved.address.clone(),
    };
    let (name, plugins) = match source.section.value.as_ref() {
        None => (None, Vec::new()),
        Some(Contents::Repo { manifest, icon }) => (
            None,
            vec![AddedPlugin {
                source: saved.address.clone(),
                provider: provider.clone(),
                via_index: None,
                manifest: (**manifest).clone(),
                icon: icon.clone(),
            }],
        ),
        Some(Contents::Index { index, icons }) => {
            let location = match &target {
                Some(Target::Index(location)) => Some(location),
                _ => None,
            };
            let plugins = index
                .plugins
                .iter()
                .filter_map(|listed| {
                    let id = listed.manifest.id.to_string();
                    Some(AddedPlugin {
                        source: location?.item_source(&id),
                        provider: format!("github.com/{}", listed.repo),
                        via_index: Some(provider.clone()),
                        manifest: listed.manifest.clone(),
                        icon: icons.get(&id).cloned(),
                    })
                })
                .collect();
            (index.name.clone(), plugins)
        }
    };
    AddedView {
        address: saved.address.clone(),
        kind: match saved.kind {
            SavedKind::Repo => SourceKind::Repo,
            SavedKind::Index => SourceKind::Index,
        },
        provider,
        name,
        repo: match target {
            Some(Target::Repo(s)) => Some(s.repo),
            _ => None,
        },
        plugins,
        fetched_at_ms: source.section.fetched_at_ms,
        problem: source.section.problem.clone(),
        unreadable: match &source.section.value {
            Some(Contents::Index { index, .. }) => index.skipped.clone(),
            _ => Vec::new(),
        },
    }
}
