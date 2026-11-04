//! 商店给 App 的视图：列表（`GET /_wonton/api/store`）和来源最新版本的详情（`GET /_wonton/api/store/plugin`）。

use wonton_protocol::{PluginDetail, PluginTrust, StoreItem, StoreListing, StoreSection};

use super::super::gallery::IconSource;
use super::super::github_source::{SourceRef, Trust};
use super::super::install_error::InstallError;
use super::super::languages::Languages;
use super::super::manager::{PluginManager, supported};
use super::super::manifest::{Arch, Manifest, PluginKind};
use super::super::package::PluginPackage;
use super::super::package_files::normalize;
use super::super::views::{author, category, kind, permission_line, requirement_lines};
use super::index::Listed;

/// 列表和详情里一个插件共有的部分。
struct Shown<'a> {
    source: String,
    manifest: &'a Manifest,
    icon: Option<IconSource>,
    trust: PluginTrust,
    stars: Option<u32>,
}

impl PluginManager {
    /// 商店：官方插件按索引里的顺序，社区插件按星数。`refresh` 时不等缓存过期，直接去来源取。
    pub async fn store_listing(&self, langs: &Languages, refresh: bool) -> StoreListing {
        let (index, official_at, official_problem) = self.storefront.official(refresh).await;
        let (community, community_at, community_problem) =
            self.storefront.community(&index, refresh).await;
        let arch = self.engine.host_arch().await.ok();
        let mut icons = Vec::new();
        let mut official = Vec::new();
        for listed in &index.plugins {
            let icon = self.official_icon(listed).await;
            icons.extend(icon.clone());
            let shown = Shown {
                source: listed.entry.source.clone(),
                manifest: &listed.manifest,
                icon,
                trust: PluginTrust::Official,
                stars: None,
            };
            official.push(self.item(&shown, arch.as_deref(), langs));
        }
        let community = community
            .iter()
            .map(|c| {
                icons.extend(c.icon.clone());
                let shown = Shown {
                    source: format!("github:{}", c.repo),
                    manifest: &c.manifest,
                    icon: c.icon.clone(),
                    trust: PluginTrust::Community,
                    stars: Some(c.stars),
                };
                self.item(&shown, arch.as_deref(), langs)
            })
            .collect();
        self.gallery.remember_store(icons);
        StoreListing {
            official: StoreSection {
                plugins: official,
                fetched_at_ms: official_at,
                problem: official_problem,
            },
            community: StoreSection {
                plugins: community,
                fetched_at_ms: community_at,
                problem: community_problem,
            },
        }
    }

    /// 来源最新的版本的详情：官方插件是索引里的版本，社区插件是最新的正式版本。
    pub async fn store_plugin(
        &self,
        source: &str,
        langs: &Languages,
    ) -> Result<PluginDetail, InstallError> {
        let wanted = SourceRef::parse(source)?;
        let (index, _, _) = self.storefront.official(false).await;
        let package = match index.find(&wanted).filter(|l| l.matches(&wanted)) {
            Some(listed) => self.storefront.files().package(listed).await?,
            None => self.fetch_source(source).await?,
        };
        let stars = self.storefront.stars(&wanted.repo).await;
        let icon = self.check_icon(&package).await?;
        if let Some(icon) = &icon {
            self.gallery.remember_preview(icon);
        }
        Ok(self.detail(&package, icon, stars, langs).await)
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
        let arch = self.engine.host_arch().await.ok();
        let installed = self.installed_plugin(&m.id, langs);
        PluginDetail {
            source: origin.map(|o| o.repo.clone()),
            id: m.id.to_string(),
            title: langs.pick_owned(&m.title),
            summary: langs.pick_owned(&m.description),
            about: m.about.as_ref().map(|t| langs.pick_owned(t)),
            icon: icon.map(|i| i.paths()),
            screenshots: Vec::new(),
            category: m.category.map(category),
            tags: m.tags.clone(),
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
            permissions: m.permissions.iter().map(permission_line).collect(),
            requirements: requirement_lines(m),
            has_web: m.webui.is_some(),
            installed,
            blocked: blocked(m, arch.as_deref()),
            // 从商店点进已装的插件也是同一个详情页，开关跟着装着的版本走。
            lan_webui: self
                .installed_detail(&m.id, langs)
                .and_then(|detail| detail.lan_webui),
            save_grants: self.save_grants(&m.id),
        }
    }

    fn item(&self, shown: &Shown, arch: Option<&str>, langs: &Languages) -> StoreItem {
        let m = shown.manifest;
        let installed_version = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins.get(&m.id).map(|e| e.version.clone())
        };
        StoreItem {
            source: shown.source.clone(),
            id: m.id.to_string(),
            title: langs.pick_owned(&m.title),
            summary: langs.pick_owned(&m.description),
            icon: shown.icon.as_ref().map(IconSource::paths),
            category: m.category.map(category),
            tags: m.tags.clone(),
            kind: kind(m.kind),
            version: m.version.clone(),
            trust: shown.trust,
            author: author(m),
            stars: shown.stars,
            installed_version,
            high_permissions: m.permissions.iter().any(|p| p.is_high()),
            blocked: blocked(m, arch),
        }
    }

    /// 官方插件的图标：按索引记的 sha256 取来核对；取过的不再取。取不到时列出时没有图标，日志里记下原因。
    async fn official_icon(&self, listed: &Listed) -> Option<IconSource> {
        let path = normalize(listed.manifest.icon.as_deref()?).ok()?;
        let sha256 = listed.entry.files.get(&path)?;
        if let Some(icon) = self.gallery.preview(sha256) {
            return Some(icon);
        }
        match self.storefront.files().file(listed, &path).await {
            Ok(data) => IconSource::from_file(&path, data),
            Err(e) => {
                tracing::warn!(source = %listed.entry.source, error = %e, "官方插件的图标取不到");
                None
            }
        }
    }
}

/// 这台主机上装不了的原因：系统插件、核心不支持的权限、宿主架构不符。查不到宿主架构（Docker 不可用）时不按架构判断。
fn blocked(m: &Manifest, arch: Option<&str>) -> Option<String> {
    if m.kind == PluginKind::System {
        return Some("系统插件只能随核心预装".to_owned());
    }
    if let Some(p) = m.permissions.iter().find(|p| !supported(p)) {
        return Some(format!("核心还不支持权限 {p}"));
    }
    let wanted = m.requirements.as_ref().and_then(|r| r.arch.as_ref())?;
    let arch = arch?;
    match Arch::from_docker(arch) {
        Some(host) if wanted.contains(&host) => None,
        Some(_) => Some(format!("插件不支持本机架构 {arch}")),
        None => Some(format!("无法识别本机架构 {arch}")),
    }
}
