//! 升级已装的插件（ADR-0017 的“升级”、插件规范的“升级”）：检查新版本（[`offers`]），预览，主人确认后换容器（[`run`]）。
//! 按预览给的摘要升级：主人确认的就是装上的，来源在这之间变了就要重新预览、重新确认。

mod offers;
mod run;

use std::sync::Arc;

pub use offers::Upgrades;
use wonton_protocol::{CardLine, InstallPreview, UpgradeRequest};

use super::github_source::{self, SourceRef, Trust, Wanted};
use super::lan::webui::{open_ports, switched};
use super::languages::Languages;
use super::manager::{InstallError, PluginManager, detached};
use super::manifest::{Permission, PluginId};
use super::package::PluginPackage;
use super::store::InstalledRecord;
use super::storefront::index::{Index, Listed as IndexListed};

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
        let (index, problem) = self.official_index().await;
        if let Some(listed) = self.official_entry(&index, record) {
            if !newer(&listed.manifest.version, &record.version) {
                return Err(none());
            }
            return self.official_package(listed).await;
        }
        let Some(origin) = &record.origin else {
            let no_index = problem
                .map(|p| format!("：{}", p.details))
                .unwrap_or_default();
            return Err(InstallError::NotFound(format!(
                "插件 {id} 没有来源，查不到新版本{no_index}"
            )));
        };
        // 官方索引不再收录时不改从 GitHub 升级：那样信任级别就悄悄降了。
        if origin.trust == Trust::Official {
            return Err(none());
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

    /// 上传的新版本：`id` 不能变，没有降级。开发时同一个版本号可以反复上传。
    fn check_upload(package: &PluginPackage, record: &InstalledRecord) -> Result<(), InstallError> {
        let manifest = &package.manifest;
        if manifest.id != record.id {
            return Err(InstallError::Invalid(format!(
                "插件包是 {}，要升级的是 {}：升级不能改 id",
                manifest.id, record.id
            )));
        }
        if newer(&record.version, &manifest.version) {
            return Err(InstallError::Invalid(format!(
                "插件包的版本 {} 比装着的 {} 旧：没有降级",
                manifest.version, record.version
            )));
        }
        Ok(())
    }

    /// 升级预览：`upload` 是上传的插件包，没有时取能升到的版本。什么也不改。
    pub async fn preview_upgrade(
        &self,
        id: &PluginId,
        upload: Option<PluginPackage>,
        langs: &Languages,
    ) -> Result<InstallPreview, InstallError> {
        let record = self.upgradable_record(id)?;
        let package = match upload {
            Some(package) => {
                Self::check_upload(&package, &record)?;
                package
            }
            None => self.latest_package(&record).await?,
        };
        let blocked = self.host_block(&package.manifest).await?;
        let mut preview = self.preview_with(&package, langs, blocked).await?;
        preview.upgrade_from = Some(record.version.clone());
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
        if request.source.is_some() {
            return Err(InstallError::Unsupported(
                "这个版本的核心只能升到提示的版本".to_owned(),
            ));
        }
        let record = self.upgradable_record(id)?;
        let package = self.latest_package(&record).await?;
        let digest = package.digest();
        if digest != request.digest {
            return Err(InstallError::Changed(format!(
                "{id} 的新版本在预览之后变了（插件包摘要现在是 {digest}），要重新预览、重新确认"
            )));
        }
        self.upgrade_detached(package, granted, request.lan_webui)
            .await
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
        Self::check_upload(&package, &record)?;
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
