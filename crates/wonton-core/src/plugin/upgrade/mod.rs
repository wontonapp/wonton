//! 升级已装的插件（ADR-0017 的“升级”、插件规范的“升级”）：检查新版本（[`offers`]），预览，主人确认后换容器（[`run`]）。
//! 按预览给的摘要升级：主人确认的就是装上的，来源在这之间变了就要重新预览、重新确认。

mod adopt;
mod offers;
mod run;

use std::sync::Arc;

pub use offers::Upgrades;
use wonton_protocol::{CardLine, InstallPreview, StoreFailure, StoreProblem, UpgradeRequest};

use super::github_source::{self, Origin, SourceRef, Trust, Wanted};
use super::lan::webui::{open_ports, switched};
use super::languages::Languages;
use super::manager::{InstallError, PluginManager, detached};
use super::manifest::{Permission, PluginId};
use super::package::PluginPackage;
use super::source_address::chosen_source;
use super::store::InstalledRecord;
use super::storefront::index::{Index, Listed as IndexListed};

/// 升到哪里。
pub enum UpgradeTo {
    /// 能升到的版本（官方索引、仓库最新的 tag 或第三方索引里的）。
    Offered,
    /// 主人选的来源。
    Source(String),
    /// 上传的插件包。
    Upload(Box<PluginPackage>),
}

/// 这个包在哪里来的，给主人看。
fn origin_label(origin: Option<&Origin>) -> String {
    // 同一个仓库可以直接装，也可以经索引装，只写提供方的话换来源的提示前后一样。
    origin.map_or_else(
        || "你上传的文件".to_owned(),
        |o| {
            o.via_index()
                .map_or_else(|| o.provider(), |via| format!("{via} 的索引"))
        },
    )
}

/// 装着的和这个包来自同一个地方：以后按同一个来源提示升级。
fn same_source(record: &InstalledRecord, package: &PluginPackage) -> bool {
    let place = |o: &Origin| (o.repo.clone(), o.index.clone());
    record.origin.as_ref().map(place) == package.origin.as_ref().map(place)
}

/// 主人主动换版本时要醒目提示的：比装着的旧、来源变了。
fn chosen_warnings(record: &InstalledRecord, package: &PluginPackage) -> Vec<CardLine> {
    let warning = |text: String| CardLine {
        text,
        emphasis: true,
    };
    let mut warnings = Vec::new();
    let version = &package.manifest.version;
    if older(version, &record.version) {
        warnings.push(warning(format!(
            "比装着的 {} 旧。应用的数据可能已经按新版本迁移过，旧版本可能读不了，先备份",
            record.version
        )));
    }
    if !same_source(record, package) {
        warnings.push(warning(format!(
            "来源从 {} 换成 {}，换了之后云吞不再按原来的来源提示升级",
            origin_label(record.origin.as_ref()),
            origin_label(package.origin.as_ref())
        )));
    }
    warnings
}

/// `new` 比 `installed` 旧。读不懂版本号的不算：只有说得清才提示。
fn older(new: &str, installed: &str) -> bool {
    match (
        semver::Version::parse(new),
        semver::Version::parse(installed),
    ) {
        (Ok(new), Ok(installed)) => new < installed,
        _ => false,
    }
}

/// `new` 比 `installed` 新，而且是正式版本。读不懂版本号的不算：只升不降，说不清就不升。
fn newer(new: &str, installed: &str) -> bool {
    match (
        semver::Version::parse(new),
        semver::Version::parse(installed),
    ) {
        (Ok(new), Ok(installed)) => new.pre.is_empty() && new > installed,
        _ => false,
    }
}

/// 记录里的插件包。
fn installed_package(record: &InstalledRecord) -> Result<PluginPackage, InstallError> {
    let mut package = PluginPackage::with_files(&record.source, record.files.clone())?;
    package.origin = record.origin.clone();
    Ok(package)
}

/// 官方索引这次没取到：上次的索引里没有新版本，不等于真的没有。
fn index_unavailable(problem: StoreProblem) -> InstallError {
    let details = format!("官方索引没取到，查不了新版本：{}", problem.details);
    match problem.reason {
        StoreFailure::Unreachable | StoreFailure::Timeout | StoreFailure::RateLimited => {
            InstallError::SourceUnavailable(details)
        }
        StoreFailure::IndexTooNew => InstallError::Unsupported(details),
        _ => InstallError::UpstreamUnavailable(details),
    }
}

/// 按提交装的固定在那个提交上，不检查新版本。
fn pinned(record: &InstalledRecord) -> bool {
    record.origin.as_ref().is_some_and(|o| o.tag.is_none())
}

/// 新版本让开关管着的局域网端口，旧版本没有开关时是开着的：开关算关，升级后关掉，主人可以在确认页上打开（ADR-0023）。
fn closing_ports(
    old: &PluginPackage,
    record: &InstalledRecord,
    new: &PluginPackage,
) -> Option<CardLine> {
    let switched_ports = switched(new).filter(|_| switched(old).is_none())?.ports;
    let open = open_ports(old, record.lan_webui);
    let closing: Vec<String> = switched_ports
        .iter()
        .filter(|p| open.contains(p))
        .map(|p| p.key())
        .collect();
    (!closing.is_empty()).then(|| CardLine {
        text: format!(
            "局域网端口 {} 升级后会关掉，要继续开着就打开下面的开关",
            closing.join("、")
        ),
        emphasis: true,
    })
}

impl PluginManager {
    /// 官方索引里对应这个插件的一项：按来源装的看仓库；随核心预装的没有来源，看插件 ID。上传的不算。
    fn official_entry<'a>(
        &self,
        index: &'a Index,
        record: &InstalledRecord,
    ) -> Option<&'a IndexListed> {
        match &record.origin {
            // 经第三方索引装的只看那个索引，官方也收录了同一个仓库也一样（ADR-0031）。
            Some(origin) if origin.index.is_some() => None,
            Some(origin) => index.find(&SourceRef::parse(&origin.repo).ok()?),
            None => {
                let bundled = self.bundled.read().expect("插件表锁中毒");
                let id = &record.id;
                let by_id = |p: &&IndexListed| p.manifest.id == *id;
                bundled
                    .contains(id)
                    .then(|| index.plugins.iter().find(by_id))?
            }
        }
    }

    /// 能升级的插件的记录。系统插件随核心升级，接口升不了。
    fn upgradable_record(&self, id: &PluginId) -> Result<InstalledRecord, InstallError> {
        if self.is_system(id) {
            return Err(InstallError::Unsupported("系统插件随核心升级".to_owned()));
        }
        self.store
            .load(id)?
            .ok_or_else(|| InstallError::NotInstalled(id.to_string()))
    }

    /// 能升到的版本的插件包。没有新版本时是 `not_found`。
    async fn latest_package(
        &self,
        record: &InstalledRecord,
    ) -> Result<PluginPackage, InstallError> {
        let id = &record.id;
        let none = || InstallError::NotFound(format!("插件 {id} 没有新版本"));
        if pinned(record) {
            return Err(none());
        }
        // 主人在等结果：不等缓存过期。这次没取到时上次的索引说不清有没有新版本，如实报没取到。
        let (index, problem) = self.refreshed_official_index(true).await;
        let none_or_unknown = || problem.clone().map_or_else(none, index_unavailable);
        if let Some(listed) = self.official_entry(&index, record) {
            if !newer(&listed.manifest.version, &record.version) {
                return Err(none_or_unknown());
            }
            return self.official_package(listed).await;
        }
        let Some(origin) = &record.origin else {
            if self.expects_official(record) {
                return Err(none_or_unknown());
            }
            return Err(InstallError::NotFound(format!(
                "插件 {id} 没有来源，查不到新版本"
            )));
        };
        if let Some(source) = &origin.index {
            let package = self.fetch_source(source).await?;
            if !newer(&package.manifest.version, &record.version) {
                return Err(none());
            }
            return Ok(package);
        }
        // 官方索引不再收录时不改从 GitHub 升级：那样信任级别就悄悄降了。
        if origin.trust == Trust::Official {
            return Err(none_or_unknown());
        }
        let source = SourceRef {
            repo: SourceRef::parse(&origin.repo)?.repo,
            wanted: Wanted::Latest,
        };
        let latest = github_source::resolve(&self.github, &source).await?;
        let package = github_source::fetch(&self.github, &latest).await?;
        if !newer(&package.manifest.version, &record.version) {
            return Err(none());
        }
        Ok(package)
    }

    /// 主人主动选的版本（ADR-0031）：`id` 不能变。可以比装着的旧，也可以换来源，预览里醒目提示。
    fn check_same_plugin(
        package: &PluginPackage,
        record: &InstalledRecord,
    ) -> Result<(), InstallError> {
        let manifest = &package.manifest;
        if manifest.id != record.id {
            return Err(InstallError::Invalid(format!(
                "插件包是 {}，要升级的是 {}：升级不能改 id",
                manifest.id, record.id
            )));
        }
        Ok(())
    }

    /// 升级预览：默认取能升到的版本，也可以是主人选的来源或者上传的插件包。什么也不改。
    pub async fn preview_upgrade(
        &self,
        id: &PluginId,
        to: UpgradeTo,
        langs: &Languages,
    ) -> Result<InstallPreview, InstallError> {
        let record = self.upgradable_record(id)?;
        let chosen = !matches!(to, UpgradeTo::Offered);
        let upload = matches!(to, UpgradeTo::Upload(_));
        let package = match to {
            UpgradeTo::Offered => self.latest_package(&record).await?,
            UpgradeTo::Source(source) => self.fetch_source(&chosen_source(&source)?).await?,
            UpgradeTo::Upload(package) => *package,
        };
        if chosen {
            Self::check_same_plugin(&package, &record)?;
            let installed = installed_package(&record)?;
            // 内容一样但来源不同的要放行：装它的索引删了之后，主人靠这个改跟仓库走，不用等上游出新版本。
            if installed.digest() == package.digest() && same_source(&record, &package) {
                return Err(InstallError::Invalid(format!(
                    "和装着的 {} 一样，不用换",
                    record.version
                )));
            }
        }
        let blocked = self.host_block(&package.manifest).await?;
        let mut preview = self
            .preview_with(&package, langs, blocked, record.written_by.clone())
            .await?;
        preview.upgrade_from = Some(record.version.clone());
        if upload {
            preview.source = Some(self.staged.stage(package.clone()));
        }
        let warnings = chosen_warnings(&record, &package);
        preview.notices.splice(0..0, warnings);
        for line in &mut preview.permissions {
            let granted = &record.granted_permissions;
            line.added = !granted.iter().any(|p| p.to_string() == line.permission);
        }
        let old = installed_package(&record)?;
        preview
            .notices
            .extend(closing_ports(&old, &record, &package));
        preview.lan_webui = switched(&package).map(|s| {
            let kept = record.lan_webui && switched(&old).is_some();
            s.offer(kept)
        });
        preview.always_allowed = self.always_allowed(id, langs);
        Ok(preview)
    }

    /// 按预览给的摘要升级。升级本身在独立的任务里执行，请求断开了也照样做完。
    pub async fn upgrade_source(
        self: &Arc<Self>,
        id: &PluginId,
        request: &UpgradeRequest,
    ) -> Result<(), InstallError> {
        let granted = request
            .grant
            .iter()
            .map(|p| p.parse::<Permission>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| InstallError::Invalid(e.to_string()))?;
        let record = self.upgradable_record(id)?;
        let package = match &request.source {
            Some(source) => {
                let package = self.fetch_source(source).await?;
                Self::check_same_plugin(&package, &record)?;
                package
            }
            None => self.latest_package(&record).await?,
        };
        let digest = package.digest();
        if digest != request.digest {
            return Err(InstallError::Changed(format!(
                "{id} 的新版本在预览之后变了（插件包摘要现在是 {digest}），要重新预览、重新确认"
            )));
        }
        self.upgrade_detached(package, granted, request.lan_webui)
            .await?;
        self.staged.remove(&digest);
        Ok(())
    }

    /// 升到上传的插件包（开发时用）。开关开着的继续开着，`lan_webui` 时打开。
    pub async fn upgrade_upload(
        self: &Arc<Self>,
        id: &PluginId,
        package: PluginPackage,
        granted: Vec<Permission>,
        lan_webui: bool,
    ) -> Result<(), InstallError> {
        let record = self.upgradable_record(id)?;
        Self::check_same_plugin(&package, &record)?;
        self.upgrade_detached(package, granted, lan_webui || record.lan_webui)
            .await
    }

    async fn upgrade_detached(
        self: &Arc<Self>,
        package: PluginPackage,
        granted: Vec<Permission>,
        lan_webui: bool,
    ) -> Result<(), InstallError> {
        let this = Arc::clone(self);
        detached(async move { this.upgrade_checked(package, &granted, lan_webui).await }).await
    }
}
