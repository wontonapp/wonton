//! 商店给 App 的视图：列表（`GET /_wonton/api/store`）和来源最新版本的详情（`GET /_wonton/api/store/plugin`）。

use wonton_protocol::{
    AddedSource, PluginDetail, PluginTrust, StoreItem, StoreListing, StoreSection,
};

use super::super::gallery::IconSource;
use super::super::github::Repo;
use super::super::github_source::{INDEX_SCHEME, Origin, SourceRef, Trust};
use super::super::host_check;
use super::super::host_facts::HostFacts;
use super::super::install_error::InstallError;
use super::super::languages::Languages;
use super::super::manager::{PluginManager, supported};
use super::super::manifest::{Manifest, PluginKind};
use super::super::package::PluginPackage;
use super::super::package_files::normalize;
use super::super::views::{
    author, cross_origin_isolation_notice, kind, permission_line, requirement_checks,
};
use super::added::AddedView;
use super::index::{Index, Listed};

/// 列表和详情里一个插件共有的部分。
struct Shown<'a> {
    source: String,
    provider: String,
    via_index: Option<String>,
    manifest: &'a Manifest,
    icon: Option<IconSource>,
    trust: PluginTrust,
    stars: Option<u32>,
}

impl PluginManager {
    /// 商店：官方插件按索引里的顺序，社区插件按星数。`refresh` 时不等缓存过期，直接去来源取。
    pub async fn store_listing(&self, langs: &Languages, refresh: bool) -> StoreListing {
        let (index, official_at, official_problem) = self.storefront.official(refresh, langs).await;
        let (community, community_at, community_problem) =
            self.storefront.community(&index, refresh, langs).await;
        self.storefront.added.refresh(refresh, langs).await;
        let added = self.storefront.added.views();
        let facts = self.host_facts(langs).await;
        let (official, mut icons) = self.official_items(&index, &facts, langs).await;
        // 主人添加过的只在“我添加的”里列一次。
        let added_repos: Vec<&Repo> = added.iter().filter_map(|a| a.repo.as_ref()).collect();
        let community_plugins = community
            .plugins
            .iter()
            .filter(|c| !added_repos.contains(&&c.repo))
            .map(|c| {
                icons.extend(c.icon.clone());
                let shown = Shown {
                    source: format!("github:{}", c.repo),
                    provider: format!("github.com/{}", c.repo),
                    via_index: None,
                    manifest: &c.manifest,
                    icon: c.icon.clone(),
                    trust: PluginTrust::Community,
                    stars: Some(c.stars),
                };
                self.item(&shown, &facts, langs)
            })
            .collect();
        let mut sections = Vec::with_capacity(added.len());
        for source in &added {
            icons.extend(source.plugins.iter().filter_map(|p| p.icon.clone()));
            sections.push(self.added_section(source, langs).await);
        }
        self.gallery.remember_store(icons);
        StoreListing {
            official: StoreSection {
                plugins: official,
                fetched_at_ms: official_at,
                problem: official_problem,
                unreadable: index.skipped.clone(),
            },
            added: sections,
            community: StoreSection {
                plugins: community_plugins,
                fetched_at_ms: community_at,
                problem: community_problem,
                unreadable: community.unreadable,
            },
        }
    }

    /// 添加的一个来源，App 上的一段。
    pub(in crate::plugin) async fn added_section(
        &self,
        source: &AddedView,
        langs: &Languages,
    ) -> AddedSource {
        let facts = self.host_facts(langs).await;
        let plugins = source
            .plugins
            .iter()
            .map(|p| {
                let shown = Shown {
                    source: p.source.clone(),
                    provider: p.provider.clone(),
                    via_index: p.via_index.clone(),
                    manifest: &p.manifest,
                    icon: p.icon.clone(),
                    trust: PluginTrust::Community,
                    stars: None,
                };
                self.item(&shown, &facts, langs)
            })
            .collect();
        AddedSource {
            address: source.address.clone(),
            kind: source.kind,
            provider: source.provider.clone(),
            name: source.name.clone(),
            section: StoreSection {
                plugins,
                fetched_at_ms: source.fetched_at_ms,
                problem: source.problem.clone(),
                unreadable: source.unreadable.clone(),
            },
        }
    }

    /// 引导使用的官方插件清单，不刷新社区来源。
    pub async fn onboarding_store_listing(
        &self,
        langs: &Languages,
    ) -> (
        super::index::Index,
        Option<wonton_protocol::StoreProblem>,
        Vec<StoreItem>,
    ) {
        let (index, _, problem) = self.storefront.official(false, langs).await;
        let facts = self.host_facts(langs).await;
        let (items, icons) = self.official_items(&index, &facts, langs).await;
        self.gallery.remember_store(icons);
        (index, problem, items)
    }

    /// 来源最新的版本的详情：官方插件是索引里的版本，社区插件是最新的正式版本。
    pub async fn store_plugin(
        &self,
        source: &str,
        langs: &Languages,
    ) -> Result<PluginDetail, InstallError> {
        let (package, stars) = if source.starts_with(INDEX_SCHEME) {
            (self.fetch_source(source).await?, None)
        } else {
            let wanted = SourceRef::parse(source, langs)?;
            let (index, _, _) = self.storefront.official(false, langs).await;
            let package = match index.find(&wanted).filter(|l| l.matches(&wanted)) {
                Some(listed) => self.storefront.files().package(listed, langs).await?,
                None => self.fetch_source(source).await?,
            };
            (package, self.storefront.stars(&wanted.repo).await)
        };
        let icon = self.check_icon(&package, langs).await?;
        if let Some(icon) = &icon {
            self.gallery.remember_preview(icon);
        }
        let mut detail = self.detail(&package, icon, stars, langs).await;
        // 详情页上的“获取”装的就是列表上的这一个：添加来源时可以固定版本。
        detail.source = Some(source.to_owned());
        Ok(detail)
    }

    async fn detail(
        &self,
        package: &PluginPackage,
        icon: Option<IconSource>,
        stars: Option<u32>,
        langs: &Languages,
    ) -> PluginDetail {
        let m = &package.manifest;
        let origin = package.origin.as_ref();
        let facts = self.host_facts(langs).await;
        let installed = self.installed_plugin(&m.id, langs);
        PluginDetail {
            source: origin.map(|o| o.index.clone().unwrap_or_else(|| o.repo.clone())),
            provider: origin.map(Origin::provider),
            via_index: origin.and_then(Origin::via_index),
            id: m.id.to_string(),
            title: langs.pick_owned(&m.title),
            summary: langs.pick_owned(&m.description),
            about: m.about.as_ref().map(|t| langs.pick_owned(t)),
            icon: icon.map(|i| i.paths()),
            screenshots: Vec::new(),
            tags: langs.pick_tags(&m.tags),
            kind: kind(m.kind),
            version: m.version.clone(),
            trust: match origin.map(|o| o.trust) {
                Some(Trust::Official) => PluginTrust::Official,
                _ => PluginTrust::Community,
            },
            author: author(m),
            homepage: m.homepage.clone(),
            repository: origin.map(|o| o.repository_url()),
            license: m.license.clone(),
            stars,
            release_notes: m.release_notes.as_ref().map(|t| langs.pick_owned(t)),
            permissions: m
                .permissions
                .iter()
                .map(|p| permission_line(p, langs))
                .collect(),
            requirements: requirement_checks(m, &facts, langs),
            secret_inputs: Vec::new(),
            has_web: m.webui.is_some(),
            installed,
            blocked: blocked(m, &facts, langs),
            notices: cross_origin_isolation_notice(m, langs)
                .into_iter()
                .collect(),
            // 从商店点进已装的插件也是同一个详情页：轻提示、局域网开关和资源上限都跟着装着的版本走。
            hints: self
                .installed_detail(&m.id, langs)
                .map(|detail| detail.hints)
                .unwrap_or_default(),
            missing_requires: Vec::new(),
            missing_wants: Vec::new(),
            capability_providers: Vec::new(),
            lan_webui: self
                .with_lan_client(
                    &m.id,
                    self.installed_detail(&m.id, langs)
                        .and_then(|detail| detail.lan_webui),
                )
                .await,
            save_grants: self.save_grants(&m.id),
            ask_every_update: None,
            limits: self
                .installed_detail(&m.id, langs)
                .and_then(|detail| detail.limits),
            shares: None,
        }
    }

    fn item(&self, shown: &Shown, facts: &HostFacts, langs: &Languages) -> StoreItem {
        let m = shown.manifest;
        let installed_version = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins.get(&m.id).map(|e| e.version.clone())
        };
        StoreItem {
            source: shown.source.clone(),
            provider: Some(shown.provider.clone()),
            via_index: shown.via_index.clone(),
            id: m.id.to_string(),
            title: langs.pick_owned(&m.title),
            summary: langs.pick_owned(&m.description),
            icon: shown.icon.as_ref().map(IconSource::paths),
            tags: langs.pick_tags(&m.tags),
            kind: kind(m.kind),
            version: m.version.clone(),
            trust: shown.trust,
            author: author(m),
            stars: shown.stars,
            installed_version,
            high_permissions: m.permissions.iter().any(|p| p.is_high()),
            blocked: blocked(m, facts, langs),
        }
    }

    /// 官方插件的图标：按索引记的 sha256 取来核对；取过的不再取。取不到时列出时没有图标，日志里记下原因。
    async fn official_icon(&self, listed: &Listed) -> Option<IconSource> {
        let path = normalize(listed.manifest.icon.as_deref()?).ok()?;
        let sha256 = listed.entry.files.get(&path)?;
        if let Some(icon) = self.gallery.preview(sha256) {
            return Some(icon);
        }
        match self
            .storefront
            .files()
            .file(listed, &path, &self.owner_languages())
            .await
        {
            Ok(data) => IconSource::from_file(&path, data),
            Err(e) => {
                tracing::warn!(source = %listed.entry.source, error = %e, "官方插件的图标取不到");
                None
            }
        }
    }
    async fn official_items(
        &self,
        index: &Index,
        facts: &HostFacts,
        langs: &Languages,
    ) -> (Vec<StoreItem>, Vec<IconSource>) {
        let mut icons = Vec::new();
        let mut items = Vec::with_capacity(index.plugins.len());
        for listed in &index.plugins {
            let icon = self.official_icon(listed).await;
            if let Some(icon) = &icon {
                icons.push(icon.clone());
            }
            let shown = Shown {
                source: listed.entry.source.clone(),
                provider: listed.entry.source.replacen("github:", "github.com/", 1),
                via_index: None,
                manifest: &listed.manifest,
                icon,
                trust: PluginTrust::Official,
                stars: None,
            };
            items.push(self.item(&shown, facts, langs));
        }
        (items, icons)
    }
}

/// 这台主机上装不了的原因：系统插件、核心不支持的权限、不满足的要求。查不出来的要求不拦。
fn blocked(m: &Manifest, facts: &HostFacts, langs: &Languages) -> Option<String> {
    if m.kind == PluginKind::System {
        return Some(crate::l10n::text(
            langs,
            "plugin-settings-store-system-app",
            &[],
        ));
    }
    if let Some(p) = m.permissions.iter().find(|p| !supported(p)) {
        return Some(crate::l10n::text(
            langs,
            "plugin-settings-store-permission-unsupported",
            &[("permission", p.to_string().into())],
        ));
    }
    host_check::refusal(m, facts, langs).map(|refusal| refusal.to_string())
}
