//! 检查新版本（ADR-0017 的“升级”）：官方插件看官方索引，社区插件看仓库最新的正式版本 tag；每 [`CHECK_EVERY`] 一次，
//! 主人打开已安装列表或商店时也查，受同样的缓存约束；主人手动刷新商店时不等缓存过期。上传的和按提交装的不查。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::Instant;
use wonton_protocol::{CardLine, InstalledPlugins, StoreProblem, UpgradeOffer};

use super::super::github::{GitHubError, Tag};
use super::super::github_source::{SourceRef, Trust, latest};
use super::super::index_location::IndexLocation;
use super::super::install_error::InstallError;
use super::super::languages::Languages;
use super::super::manager::{PluginManager, PluginStatus};
use super::super::manifest::{LocalizedText, Manifest, PluginId};
use super::super::package::PluginPackage;
use super::super::package_files::{COMPOSE_FILE, MAX_FILE_BYTES};
use super::super::store::InstalledRecord;
use super::super::storefront::index::Index;
use super::super::storefront::{COMMUNITY_TTL, MIN_RETRY, OFFICIAL_TTL, failure};
use super::{newer, pinned};

const CHECK_EVERY: Duration = OFFICIAL_TTL;
/// 打开列表时最多等这么久：还没查完就先给上次的结果，查的那一轮接着做。
const LIST_WAIT: Duration = Duration::from_secs(3);
/// 核心启动后过这么久第一次查：先让预装目录读完，它决定哪些没有来源的插件也归官方索引管。
const FIRST_CHECK: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
struct Offer {
    version: String,
    release_notes: Option<LocalizedText>,
    permissions_changed: bool,
}

/// 一个插件查到的结果。
#[derive(Debug, Clone, Default)]
struct Checked {
    offer: Option<Offer>,
    /// 同一个版本号指向了别的提交：不算升级，如实提示。
    rewritten: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct LastCheck {
    at: Instant,
    failed: bool,
}

impl LastCheck {
    fn due(&self) -> bool {
        let gap = if self.failed {
            MIN_RETRY
        } else {
            COMMUNITY_TTL
        };
        self.at.elapsed() >= gap
    }
}

/// 上次检查的结果，和随核心带来的新版本没升成的原因。
#[derive(Default)]
pub struct Upgrades {
    checked: RwLock<HashMap<PluginId, Checked>>,
    problem: RwLock<Option<StoreProblem>>,
    bundled_failed: RwLock<HashMap<PluginId, String>>,
    /// 持有它的在查；同一时间只查一轮。
    last: tokio::sync::Mutex<Option<LastCheck>>,
}

impl Upgrades {
    pub(super) fn forget(&self, id: &PluginId) {
        self.checked.write().expect("升级检查锁中毒").remove(id);
        let mut failed = self.bundled_failed.write().expect("升级检查锁中毒");
        failed.remove(id);
    }

    pub(super) fn bundled_failed(&self, id: &PluginId, reason: String) {
        let mut failed = self.bundled_failed.write().expect("升级检查锁中毒");
        failed.insert(id.clone(), reason);
    }

    /// 列表里这个插件的升级提示和要主人知道的情况。在忙的不提示升级。
    pub(in crate::plugin) fn listing(
        &self,
        id: &PluginId,
        installed: &str,
        status: &PluginStatus,
        langs: &Languages,
    ) -> (Option<UpgradeOffer>, Vec<CardLine>) {
        let line = |text: &str| CardLine {
            text: text.to_owned(),
            emphasis: true,
        };
        let failed = self.bundled_failed.read().expect("升级检查锁中毒");
        let mut notices: Vec<CardLine> = failed.get(id).map(|r| line(r)).into_iter().collect();
        let checked = self.checked.read().expect("升级检查锁中毒");
        let Some(found) = checked.get(id) else {
            return (None, notices);
        };
        notices.extend(found.rewritten.as_deref().map(line));
        let idle = matches!(
            status,
            PluginStatus::Running | PluginStatus::Stopped | PluginStatus::Degraded { .. }
        );
        let offer = found
            .offer
            .as_ref()
            .filter(|o| idle && newer(&o.version, installed))
            .map(|o| UpgradeOffer {
                version: o.version.clone(),
                release_notes: o.release_notes.as_ref().map(|t| langs.pick_owned(t)),
                permissions_changed: o.permissions_changed,
            });
        (offer, notices)
    }

    pub(in crate::plugin) fn problem(&self) -> Option<StoreProblem> {
        self.problem.read().expect("升级检查锁中毒").clone()
    }
}

fn offer_of(manifest: &Manifest, record: &InstalledRecord) -> Offer {
    let granted = &record.granted_permissions;
    let wanted = &manifest.permissions;
    Offer {
        version: manifest.version.clone(),
        release_notes: manifest.release_notes.clone(),
        permissions_changed: wanted.len() != granted.len()
            || wanted.iter().any(|p| !granted.contains(p)),
    }
}

impl PluginManager {
    /// 已安装的插件，先查一次新版本（受缓存约束）。
    pub async fn installed_checked(self: &Arc<Self>, langs: &Languages) -> InstalledPlugins {
        if tokio::time::timeout(LIST_WAIT, self.spawn_upgrade_check(false))
            .await
            .is_err()
        {
            tracing::debug!("新版本还没查完，先给上次的结果");
        }
        self.installed(langs)
    }

    /// 在后台查一次新版本。`force` 是主人要求刷新：不等缓存过期，连官方索引一起重新取，离上次查不到一分钟的除外。
    pub fn spawn_upgrade_check(self: &Arc<Self>, force: bool) -> JoinHandle<()> {
        let this = Arc::clone(self);
        tokio::spawn(async move { this.check_upgrades(force).await })
    }

    /// 每 [`CHECK_EVERY`] 查一次新版本。
    pub fn spawn_upgrade_checks(self: &Arc<Self>) {
        let this = Arc::downgrade(self);
        tokio::spawn(async move {
            let start = Instant::now() + FIRST_CHECK;
            let mut ticks = tokio::time::interval_at(start, CHECK_EVERY);
            loop {
                ticks.tick().await;
                let Some(manager) = this.upgrade() else {
                    return;
                };
                manager.check_upgrades(false).await;
            }
        });
    }

    async fn check_upgrades(&self, force: bool) {
        let Ok(mut last) = self.upgrades.last.try_lock() else {
            return;
        };
        let wait = |l: LastCheck| {
            if force {
                l.at.elapsed() < MIN_RETRY
            } else {
                !l.due()
            }
        };
        if last.is_some_and(wait) {
            return;
        }
        let problem = self.check_round(force).await;
        *last = Some(LastCheck {
            at: Instant::now(),
            failed: problem.is_some(),
        });
    }

    /// 不管缓存查一轮，官方索引用缓存里的。
    #[cfg(test)]
    pub(in crate::plugin) async fn check_upgrades_now(&self) -> Option<StoreProblem> {
        self.check_round(false).await
    }

    /// 查一轮，返回没查成的原因；`refresh` 时官方索引不等缓存过期。一个插件没查到时留着它上次的结果。
    /// 没有要查的插件时不连外面。
    async fn check_round(&self, refresh: bool) -> Option<StoreProblem> {
        let records: Vec<InstalledRecord> = match self.store.load_all() {
            Ok(files) => files.into_iter().filter_map(|f| f.record.ok()).collect(),
            Err(e) => {
                tracing::warn!(error = %e, "读插件记录失败，这轮不查新版本");
                return None;
            }
        };
        let mut records: Vec<InstalledRecord> = records
            .into_iter()
            .filter(|r| self.has_source_to_check(r))
            .collect();
        let mut results = HashMap::new();
        let mut problem = None;
        if !records.is_empty() {
            let (index, index_problem) = self.refreshed_official_index(refresh).await;
            self.adopt_official(&index, &mut records);
            let uses_index = records.iter().any(|r| self.expects_official(r));
            for record in &records {
                let id = record.id.clone();
                match self.check_one(&index, record).await {
                    Ok(checked) => {
                        results.insert(id, checked);
                    }
                    Err(e) => {
                        tracing::warn!(plugin = %id, error = %e, "查新版本失败");
                        let previous = self.upgrades.checked.read().expect("升级检查锁中毒");
                        results.extend(previous.get(&id).cloned().map(|c| (id.clone(), c)));
                        problem.get_or_insert(StoreProblem {
                            reason: failure(&e),
                            details: format!("{id} 的新版本没查到：{e}"),
                        });
                    }
                }
            }
            problem = problem.or(index_problem.filter(|_| uses_index));
        }
        *self.upgrades.checked.write().expect("升级检查锁中毒") = results;
        *self.upgrades.problem.write().expect("升级检查锁中毒") = problem.clone();
        problem
    }

    /// 上传的、按提交装的和系统插件不查。
    fn has_source_to_check(&self, record: &InstalledRecord) -> bool {
        !self.is_system(&record.id)
            && !pinned(record)
            && (record.origin.is_some() || self.expects_official(record))
    }

    /// 这个插件的新版本要看官方索引，索引读不到就查不了。
    pub(super) fn expects_official(&self, record: &InstalledRecord) -> bool {
        match &record.origin {
            Some(origin) => origin.trust == Trust::Official && origin.tag.is_some(),
            None => self
                .bundled
                .read()
                .expect("插件表锁中毒")
                .contains(&record.id),
        }
    }

    async fn check_one(
        &self,
        index: &Index,
        record: &InstalledRecord,
    ) -> Result<Checked, GitHubError> {
        let origin = record.origin.as_ref();
        if let Some(listed) = self.official_entry(index, record) {
            let same_version = listed.entry.version == record.version;
            let moved = origin.is_some_and(|o| o.commit != listed.entry.commit);
            let version = &listed.entry.version;
            return Ok(Checked {
                offer: newer(version, &record.version).then(|| offer_of(&listed.manifest, record)),
                rewritten: (same_version && moved)
                    .then(|| format!("官方索引里版本 {version} 换了提交：不算新版本，装着的没变")),
            });
        }
        let Some(origin) = origin.filter(|o| o.trust == Trust::Community) else {
            return Ok(Checked::default());
        };
        if let Some(source) = &origin.index {
            return self.check_in_index(source, record).await;
        }
        let Ok(source) = SourceRef::parse(&origin.repo) else {
            return Ok(Checked::default());
        };
        let tags = self.github.tags(&source.repo).await?;
        let rewritten = tags
            .iter()
            .find(|t| Some(&t.name) == origin.tag.as_ref() && t.commit != origin.commit)
            .map(|t| {
                format!(
                    "这个版本的 tag {} 被作者改到了别的提交：不算新版本，装着的没变",
                    t.name
                )
            });
        let offer = match latest(tags) {
            Some(tag) if newer(tag.name.trim_start_matches('v'), &record.version) => {
                self.community_offer(&source, &tag, record).await?
            }
            _ => None,
        };
        Ok(Checked { offer, rewritten })
    }

    /// 经第三方索引装的只看那个索引：索引删了就不提示，也不改去看 GitHub 的 tag（ADR-0031）。
    async fn check_in_index(
        &self,
        source: &str,
        record: &InstalledRecord,
    ) -> Result<Checked, GitHubError> {
        let Ok((location, id)) = IndexLocation::of_item_source(source) else {
            return Ok(Checked::default());
        };
        let added = &self.storefront.added;
        if !added.has_index(&location) {
            return Ok(Checked {
                offer: None,
                rewritten: Some(format!(
                    "索引 {} 已经从商店删掉，不再提示新版本",
                    location.provider()
                )),
            });
        }
        let listed = match added.listed_in(&location, id).await {
            Ok(listed) => listed,
            Err(InstallError::SourceUnavailable(why)) => return Err(GitHubError::Unavailable(why)),
            Err(e) => {
                tracing::info!(plugin = %record.id, error = %e, "索引里查不到新版本");
                return Ok(Checked::default());
            }
        };
        let version = &listed.entry.version;
        Ok(Checked {
            offer: newer(version, &record.version).then(|| offer_of(&listed.manifest, record)),
            rewritten: None,
        })
    }

    /// 新 tag 上的清单。读不懂、版本和 tag 对不上的不提示，日志里记下原因，和商店不列出一样。
    async fn community_offer(
        &self,
        source: &SourceRef,
        tag: &Tag,
        record: &InstalledRecord,
    ) -> Result<Option<Offer>, GitHubError> {
        let skip = |why: String| {
            tracing::info!(plugin = %record.id, tag = %tag.name, reason = %why, "新版本不提示");
            Ok(None)
        };
        let compose = self
            .github
            .file(&source.repo, &tag.commit, COMPOSE_FILE, MAX_FILE_BYTES)
            .await;
        let compose = match compose {
            Err(e @ (GitHubError::NotFound(_) | GitHubError::TooBig(_))) => {
                return skip(e.to_string());
            }
            other => other?,
        };
        let Ok(compose) = String::from_utf8(compose) else {
            return skip(format!("{COMPOSE_FILE} 不是 UTF-8 文本"));
        };
        let manifest = match PluginPackage::manifest_of(&compose) {
            Ok(manifest) => manifest,
            Err(e) => return skip(format!("清单读不懂：{e}")),
        };
        if manifest.id != record.id {
            return skip(format!("清单的 id 变成了 {}", manifest.id));
        }
        if tag.name.strip_prefix('v') != Some(manifest.version.as_str()) {
            return skip(format!("清单的版本是 {}", manifest.version));
        }
        Ok(Some(offer_of(&manifest, record)))
    }
}
