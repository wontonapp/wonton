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
use super::store::{EnabledPermission, InstalledRecord};
use super::storefront::index::{Index, Listed as IndexListed};
use crate::l10n;

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
fn origin_label(origin: Option<&Origin>, langs: &Languages) -> String {
    // 同一个仓库可以直接装，也可以经索引装，只写提供方的话换来源的提示前后一样。
    origin.map_or_else(
        || crate::l10n::text(langs, "plugin-settings-upgrade-origin-upload", &[]),
        |o| {
            o.via_index().map_or_else(
                || o.provider(),
                |via| {
                    crate::l10n::text(
                        langs,
                        "plugin-settings-upgrade-origin-index",
                        &[("via", via.into())],
                    )
                },
            )
        },
    )
}

/// 装着的和这个包来自同一个地方：以后按同一个来源提示升级。
fn same_source(record: &InstalledRecord, package: &PluginPackage) -> bool {
    let place = |o: &Origin| (o.repo.clone(), o.index.clone());
    record.origin.as_ref().map(place) == package.origin.as_ref().map(place)
}

/// 主人主动换版本时要醒目提示的：比装着的旧、来源变了。
fn chosen_warnings(
    record: &InstalledRecord,
    package: &PluginPackage,
    langs: &Languages,
) -> Vec<CardLine> {
    let warning = |text: String| CardLine {
        text,
        emphasis: true,
    };
    let mut warnings = Vec::new();
    let version = &package.manifest.version;
    if older(version, &record.version) {
        warnings.push(warning(crate::l10n::text(
            langs,
            "plugin-settings-upgrade-older",
            &[("version", record.version.as_str().into())],
        )));
    }
    if !same_source(record, package) {
        warnings.push(warning(crate::l10n::text(
            langs,
            "plugin-settings-upgrade-source-changed",
            &[
                ("old", origin_label(record.origin.as_ref(), langs).into()),
                ("new", origin_label(package.origin.as_ref(), langs).into()),
            ],
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
pub(in crate::plugin) fn installed_package(
    record: &InstalledRecord,
    langs: &Languages,
) -> Result<PluginPackage, InstallError> {
    let mut package = PluginPackage::with_files(&record.source, record.files.clone(), langs)?;
    package.origin = record.origin.clone();
    Ok(package)
}

/// 官方索引这次没取到：上次的索引里没有新版本，不等于真的没有。
fn index_unavailable(problem: StoreProblem, langs: &Languages) -> InstallError {
    let details = crate::l10n::text(
        langs,
        "plugin-settings-upgrade-index-unavailable",
        &[("details", problem.details.into())],
    );
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
    langs: &Languages,
) -> Option<CardLine> {
    let switched_ports = switched(new).filter(|_| switched(old).is_none())?.ports;
    let open = open_ports(old, record.lan_webui);
    let closing: Vec<String> = switched_ports
        .iter()
        .filter(|p| open.contains(p))
        .map(|p| p.key())
        .collect();
    (!closing.is_empty()).then(|| CardLine {
        text: crate::l10n::text(
            langs,
            "plugin-settings-upgrade-lan-ports-closing",
            &[("ports", crate::l10n::list(langs, &closing).into())],
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
            Some(origin) => {
                index.find(&SourceRef::parse(&origin.repo, &Languages::default()).ok()?)
            }
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
    fn upgradable_record(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Result<InstalledRecord, InstallError> {
        if self.is_system(id) {
            return Err(InstallError::Unsupported(crate::l10n::text(
                langs,
                "plugin-settings-upgrade-system-app",
                &[],
            )));
        }
        self.store
            .load(id)?
            .ok_or_else(|| InstallError::not_installed(langs, id))
    }

    /// 能升到的版本的插件包。没有新版本时是 `not_found`。
    async fn latest_package(
        &self,
        record: &InstalledRecord,
    ) -> Result<PluginPackage, InstallError> {
        let id = &record.id;
        let langs = self.owner_languages();
        let none = || {
            InstallError::NotFound(crate::l10n::text(
                &langs,
                "plugin-settings-upgrade-none",
                &[("id", id.as_str().into())],
            ))
        };
        if pinned(record) {
            return Err(none());
        }
        // 主人在等结果：不等缓存过期。这次没取到时上次的索引说不清有没有新版本，如实报没取到。
        let (index, problem) = self.refreshed_official_index(true, &langs).await;
        let none_or_unknown = || {
            problem
                .clone()
                .map_or_else(none, |problem| index_unavailable(problem, &langs))
        };
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
            return Err(InstallError::NotFound(crate::l10n::text(
                &langs,
                "plugin-settings-upgrade-no-origin",
                &[("id", id.as_str().into())],
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
            wanted: Wanted::Latest,
            ..SourceRef::parse(&origin.repo, &langs)?
        };
        let latest = github_source::resolve(&self.github, &source, &langs).await?;
        let package = github_source::fetch(&self.github, &latest, &langs).await?;
        if !newer(&package.manifest.version, &record.version) {
            return Err(none());
        }
        Ok(package)
    }

    /// 主人主动选的版本（ADR-0031）：`id` 不能变。可以比装着的旧，也可以换来源，预览里醒目提示。
    fn check_same_plugin(
        package: &PluginPackage,
        record: &InstalledRecord,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let manifest = &package.manifest;
        if manifest.id != record.id {
            return Err(InstallError::Invalid(crate::l10n::text(
                langs,
                "plugin-settings-upgrade-id-changed",
                &[
                    ("package", manifest.id.to_string().into()),
                    ("record", record.id.to_string().into()),
                ],
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
        let record = self.upgradable_record(id, langs)?;
        let chosen = !matches!(to, UpgradeTo::Offered);
        let upload = matches!(to, UpgradeTo::Upload(_));
        let package = match to {
            UpgradeTo::Offered => self.latest_package(&record).await?,
            UpgradeTo::Source(source) => self.fetch_source(&chosen_source(&source, langs)?).await?,
            UpgradeTo::Upload(package) => *package,
        };
        if chosen {
            Self::check_same_plugin(&package, &record, langs)?;
            let installed = installed_package(&record, &self.owner_languages())?;
            // 内容一样但来源不同的要放行：装它的索引删了之后，主人靠这个改跟仓库走，不用等上游出新版本。
            if installed.digest() == package.digest() && same_source(&record, &package) {
                return Err(InstallError::Invalid(crate::l10n::text(
                    langs,
                    "plugin-settings-upgrade-same-version",
                    &[("version", record.version.as_str().into())],
                )));
            }
        }
        let blocked = self.host_block(&package.manifest, langs).await?;
        let mut preview = self
            .preview_with(
                &package,
                langs,
                blocked,
                record.written_by.clone(),
                &record.enabled_permissions,
                Some(&record),
            )
            .await?;
        preview.upgrade_from = Some(record.version.clone());
        preview.secret_inputs = self.missing_secret_inputs(id, &package.manifest, langs);
        if upload {
            preview.source = Some(self.staged.stage(package.clone()));
        }
        let warnings = chosen_warnings(&record, &package, langs);
        preview.notices.splice(0..0, warnings);
        preview.permissions = super::views_permissions::permission_lines(
            &package.manifest,
            &record.enabled_permissions,
            langs,
        );
        for line in &mut preview.permissions {
            let granted = &record.granted_permissions;
            line.added = !granted.iter().any(|p| p.to_string() == line.permission);
        }
        let old = installed_package(&record, langs)?;
        preview
            .notices
            .extend(closing_ports(&old, &record, &package, langs));
        // 以前可选、现在关着又改成必需：这个版本要主人重新确认一次。
        for permission in &package.manifest.permissions {
            let was_optional = old.manifest.optional_permissions.contains_key(permission);
            let was_enabled = record
                .enabled_permissions
                .iter()
                .any(|item| &item.permission == permission);
            if was_optional && !was_enabled {
                preview.notices.push(CardLine {
                    text: l10n::text(
                        langs,
                        "upgrade-optional-became-required",
                        &[("permission", permission.to_string().into())],
                    ),
                    emphasis: true,
                });
            }
        }
        // 以前可选、这个版本去掉了：关掉它的开关不再出现。
        for permission in old.manifest.optional_permissions.keys() {
            if !package
                .manifest
                .optional_permissions
                .contains_key(permission)
            {
                preview.notices.push(CardLine {
                    text: l10n::text(
                        langs,
                        "upgrade-optional-removed",
                        &[("permission", permission.to_string().into())],
                    ),
                    emphasis: false,
                });
            }
        }
        preview.lan_webui = switched(&package).map(|s| {
            let kept = record.lan_webui && switched(&old).is_some();
            s.offer(kept, langs)
        });
        preview.always_allowed = self.always_allowed(id, langs);
        Ok(preview)
    }

    /// 升级预览里要填的密钥：只列还没填的，已经填过的沿用。
    fn missing_secret_inputs(
        &self,
        id: &PluginId,
        manifest: &super::manifest::Manifest,
        langs: &Languages,
    ) -> Vec<wonton_protocol::SecretInput> {
        let filled = self
            .store
            .secrets()
            .load(id)
            .ok()
            .flatten()
            .unwrap_or_default();
        super::secret_input::lines(manifest, &|name| filled.contains_key(name), true, langs)
    }

    /// 按预览给的摘要升级。升级本身在独立的任务里执行，请求断开了也照样做完。
    pub async fn upgrade_source(
        self: &Arc<Self>,
        id: &PluginId,
        request: &UpgradeRequest,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let granted = super::store::parse_permissions(&request.grant)
            .map_err(|e| InstallError::Invalid(e.to_string()))?;
        let requested = super::store::parse_enabled(
            &request.enabled_permissions,
            super::store::OpenedBy::Owner,
            super::manager::now_secs(),
        )
        .map_err(|e| InstallError::Invalid(e.to_string()))?;
        let record = self.upgradable_record(id, langs)?;
        let package = match &request.source {
            Some(source) => {
                let package = self.fetch_source(source).await?;
                Self::check_same_plugin(&package, &record, langs)?;
                package
            }
            None => self.latest_package(&record).await?,
        };
        let digest = package.digest();
        if digest != request.digest {
            return Err(InstallError::Changed(crate::l10n::text(
                langs,
                "plugin-settings-upgrade-changed",
                &[("id", id.as_str().into()), ("digest", digest.into())],
            )));
        }
        let filled = self.store.secrets().load(id).ok().flatten();
        self.store_input_values(&package.manifest, &request.secrets, filled.as_ref(), langs)?;
        let settings = upgrade_values(&package, &record, &request.settings, langs)?;
        self.upgrade_detached(package, granted, requested, request.lan_webui, settings)
            .await?;
        self.staged.remove(&digest);
        Ok(())
    }

    /// 升到上传的插件包（开发时用）。开关开着的继续开着，`lan_webui` 时打开。
    #[allow(clippy::too_many_arguments)] // 装插件要的几样各管各的来源，打包反而看不清哪来的。
    pub async fn upgrade_upload(
        self: &Arc<Self>,
        id: &PluginId,
        package: PluginPackage,
        granted: Vec<Permission>,
        enabled: Vec<EnabledPermission>,
        lan_webui: bool,
        given: &wonton_protocol::SettingValues,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let record = self.upgradable_record(id, langs)?;
        Self::check_same_plugin(&package, &record, langs)?;
        let settings = upgrade_values(&package, &record, given, langs)?;
        self.upgrade_detached(
            package,
            granted,
            enabled,
            lan_webui || record.lan_webui,
            settings,
        )
        .await
    }

    async fn upgrade_detached(
        self: &Arc<Self>,
        package: PluginPackage,
        granted: Vec<Permission>,
        requested: Vec<EnabledPermission>,
        lan_webui: bool,
        settings: super::settings::Values,
    ) -> Result<(), InstallError> {
        let this = Arc::clone(self);
        detached(async move {
            this.upgrade_checked(package, &granted, &requested, lan_webui, settings)
                .await
        })
        .await
    }
}

/// 升级要用的设置值：旧值叠在请求上，再按新 Schema 校验。新版本删掉了设置时值一起丢掉。
fn upgrade_values(
    package: &PluginPackage,
    record: &InstalledRecord,
    given: &wonton_protocol::SettingValues,
    langs: &Languages,
) -> Result<super::settings::Values, InstallError> {
    let update: std::collections::HashMap<_, _> = given
        .iter()
        .map(|(name, value)| (name.clone(), Some(value.clone())))
        .collect();
    match super::settings::load(package, langs)? {
        Some(schema) => super::settings::value::merged(&schema, &record.settings, &update, langs)
            .map_err(|fields| super::settings::value_error(fields, langs)),
        None if given.is_empty() => Ok(super::settings::Values::new()),
        None => {
            let fields = given
                .keys()
                .map(|name| wonton_protocol::SettingsFieldError {
                    name: name.clone(),
                    kind: wonton_protocol::SettingsFieldProblemKind::UnknownField,
                    message: crate::l10n::text(langs, "plugin-settings-value-unknown-field", &[]),
                })
                .collect();
            Err(super::settings::value_error(fields, langs))
        }
    }
}
