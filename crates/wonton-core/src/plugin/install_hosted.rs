//! 不起容器的两种形态的装、升、卸和恢复（ADR-0080）：`static_web` 把文件放进托管目录、交给提供方；
//! `link` 只存一条记录。两种都不经渲染和部署，所以和容器插件分开走。

use std::collections::BTreeMap;

use wonton_protocol::{CapabilityProviderRef, PluginHint, PluginHintCategory};

use super::capabilities::{self, STATIC_WEB, WEB_STORAGE};
use super::hosted::HostedDirs;
use super::install_storage::{StorageNeed, required_hosted, storage_error, storage_need};
use super::languages::Languages;
use super::listed::Listed;
use super::manager::{
    Entry, Health, InstallError, PluginManager, PluginStatus, PluginView, busy, now_secs,
};
use super::manifest::{Manifest, Permission, PluginId, Runtime};
use super::package::PluginPackage;
use super::package::static_files::Problem;
use super::store::InstalledRecord;
use super::upstreams::Endpoint;
use crate::l10n;

/// 托管插件的提供方现在的样子。
pub(crate) struct Provider {
    pub id: PluginId,
    pub title: String,
    /// 运行中时网关转发的地址；没在运行或容器没有地址时为空。
    pub endpoint: Option<Endpoint>,
}

/// 不起容器的形态。
pub(super) fn is_containerless(manifest: &Manifest) -> bool {
    matches!(manifest.runtime, Runtime::StaticWeb | Runtime::Link)
}

/// 托管形态要的能力：`runtime` 的名字就是能力名（ADR-0080 第 2 节）。
pub(super) fn hosted_capability(manifest: &Manifest) -> Option<&'static str> {
    (manifest.runtime == Runtime::StaticWeb).then_some(STATIC_WEB.name)
}

/// 在插件表里找一项托管能力的提供方。同一能力只认一个：装第二个时 [`PluginManager::check_hosted_provider`] 拦下。
pub(super) fn provider_in(
    plugins: &BTreeMap<PluginId, Entry>,
    capability: &str,
    langs: &Languages,
) -> Option<Provider> {
    plugins.iter().find_map(|(id, entry)| {
        let listed = entry.listed.as_ref()?;
        let provides = listed.manifest.provides.as_ref()?;
        provides.capabilities.contains_key(capability).then(|| {
            let serving = entry.status == PluginStatus::Running
                || provides
                    .capabilities
                    .get(capability)
                    .is_some_and(|spec| entry.serving.contains(&spec.service))
                    && matches!(entry.status, PluginStatus::Degraded { .. });
            Provider {
                id: id.clone(),
                title: langs.pick_owned(&entry.title),
                endpoint: serving
                    .then(|| entry.hosted.get(capability).cloned())
                    .flatten(),
            }
        })
    })
}

/// 托管插件的状态由提供方决定：自己的记录是“运行中”时，提供方没在运行就如实标异常（ADR-0053 的“依赖的插件
/// 没有在运行”），不显示成插件自己坏了。
pub(super) fn effective_status(
    plugins: &BTreeMap<PluginId, Entry>,
    entry: &Entry,
    langs: &Languages,
) -> PluginStatus {
    let Some(listed) = entry.listed.as_ref() else {
        return entry.status.clone();
    };
    if entry.status != PluginStatus::Running {
        return entry.status.clone();
    }
    required_hosted(&listed.manifest)
        .into_iter()
        .find_map(|capability| not_serving(plugins, capability, langs))
        .unwrap_or(PluginStatus::Running)
}

/// 这项能力的提供方没在提供时的状态；在提供时为空。
fn not_serving(
    plugins: &BTreeMap<PluginId, Entry>,
    capability: &str,
    langs: &Languages,
) -> Option<PluginStatus> {
    Some(match provider_in(plugins, capability, langs) {
        Some(Provider {
            endpoint: Some(_), ..
        }) => return None,
        Some(provider) => PluginStatus::Degraded {
            reason: l10n::text(
                langs,
                "plugin-hosted-provider-not-running",
                &[("provider", provider.title.into())],
            ),
        },
        None => PluginStatus::Degraded {
            reason: l10n::text(
                langs,
                "plugin-hosted-provider-missing",
                &[("capability", capability.into())],
            ),
        },
    })
}

impl PluginManager {
    pub(super) fn hosted_dirs(&self) -> HostedDirs {
        HostedDirs::new(&self.cfg.data_dir)
    }

    /// 提供方的容器挂托管目录之前，目录要先在（否则 Docker 以 root 建一个空目录）。
    pub(super) fn prepare_hosted_roots(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let Some(provides) = &manifest.provides else {
            return Ok(());
        };
        for name in provides.capabilities.keys() {
            if capabilities::lookup(name).is_some_and(|c| c.hosted.is_some()) {
                self.hosted_dirs()
                    .ensure_capability(name)
                    .map_err(|e| storage(langs, &e))?;
            }
        }
        Ok(())
    }

    /// 托管能力的提供方，按插件表现在的样子。
    pub(crate) fn hosted_provider(&self, capability: &str) -> Option<Provider> {
        let langs = self.owner_languages();
        let plugins = self.plugins.read().expect("插件表锁中毒");
        provider_in(&plugins, capability, &langs)
    }

    /// 详情页的轻提示：托管插件的提供方没在运行（ADR-0053 的 `dependency_stopped`）。
    pub(super) fn hosted_hint(&self, manifest: &Manifest) -> Option<PluginHint> {
        let provider = required_hosted(manifest)
            .into_iter()
            .filter_map(|capability| self.hosted_provider(capability))
            .find(|provider| provider.endpoint.is_none())?;
        Some(PluginHint {
            category: PluginHintCategory::DependencyStopped,
            detail: None,
            provider: Some(CapabilityProviderRef {
                id: provider.id.to_string(),
                title: provider.title,
                high_privilege: false,
            }),
            capability: None,
        })
    }

    /// 一项托管能力只能有一个提供方：网关按能力名找它，两个时不知道转给谁。
    pub(super) fn check_hosted_provider(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let Some(provides) = &manifest.provides else {
            return Ok(());
        };
        for name in provides.capabilities.keys() {
            if capabilities::lookup(name).is_none_or(|c| c.hosted.is_none()) {
                continue;
            }
            if let Some(other) = self.hosted_provider(name).filter(|p| p.id != manifest.id) {
                return Err(InstallError::Conflict(l10n::text(
                    langs,
                    "plugin-hosted-provider-taken",
                    &[
                        ("capability", name.as_str().into()),
                        ("other", other.title.into()),
                    ],
                )));
            }
        }
        Ok(())
    }

    /// 正在用这个插件提供的托管能力的插件：卸载提供方时照 ADR-0053 拦下。
    pub(super) fn hosted_users(&self, id: &PluginId) -> Vec<String> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let Some(provides) = plugins
            .get(id)
            .and_then(|entry| entry.listed.as_ref())
            .and_then(|listed| listed.manifest.provides.clone())
        else {
            return Vec::new();
        };
        let langs = self.owner_languages();
        plugins
            .iter()
            .filter(|(other, _)| *other != id)
            .filter_map(|(_, entry)| {
                let listed = entry.listed.as_ref()?;
                required_hosted(&listed.manifest)
                    .into_iter()
                    .any(|capability| provides.capabilities.contains_key(capability))
                    .then(|| langs.pick_owned(&entry.title))
            })
            .collect()
    }

    /// 装一个不起容器的插件。调用方已核对过权限（`granted` 就是主人的确认）。
    pub(super) async fn install_containerless(
        &self,
        package: &PluginPackage,
        granted: &[Permission],
        langs: &Languages,
    ) -> Result<PluginView, InstallError> {
        let manifest = package.manifest.clone();
        if let Some(capability) = required_hosted(&manifest)
            .into_iter()
            .find(|capability| self.hosted_provider(capability).is_none())
        {
            return Err(missing_provider(capability, langs));
        }
        let listed = Listed::new(package, now_secs(), false, false, Default::default());
        self.reserve(&listed, langs)?;
        tracing::info!(plugin = %manifest.id, runtime = manifest.runtime.as_str(), "开始安装插件");
        let installed = self.publish(package, granted, None, langs).await;
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Err(e) = installed {
            plugins.remove(&manifest.id);
            drop(plugins);
            self.changed();
            tracing::warn!(plugin = %manifest.id, error = %e, "插件安装失败，已回滚");
            return Err(e);
        }
        let entry = running(listed);
        let view = self.view(&manifest.id, &entry);
        plugins.insert(manifest.id.clone(), entry);
        drop(plugins);
        self.changed();
        tracing::info!(plugin = %manifest.id, "插件安装完成");
        Ok(view)
    }

    /// 升级不起容器的插件：新文件对换进去、记录存上才算数，失败时旧版本原样留着。
    pub(super) async fn upgrade_containerless(
        &self,
        package: PluginPackage,
        granted: &[Permission],
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let id = package.manifest.id.clone();
        let old = self
            .store
            .load(&id)?
            .ok_or_else(|| InstallError::not_installed(langs, &id))?;
        let before = {
            let mut plugins = self.plugins.write().expect("插件表锁中毒");
            let entry = plugins
                .get_mut(&id)
                .ok_or_else(|| InstallError::not_installed(langs, &id))?;
            if let Some(e) = busy(&id, &entry.status, langs) {
                return Err(e);
            }
            std::mem::replace(&mut entry.status, PluginStatus::Upgrading)
        };
        self.changed();
        let result = self.publish(&package, granted, Some(&old), langs).await;
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(entry) = plugins.get_mut(&id) {
            match &result {
                Ok(()) => {
                    *entry = running(Listed::new(
                        &package,
                        old.installed_at,
                        false,
                        false,
                        Default::default(),
                    ));
                }
                Err(_) => entry.status = before,
            }
        }
        drop(plugins);
        if result.is_ok() {
            self.upgrades.forget(&id);
            tracing::info!(plugin = %id, version = %package.manifest.version, "插件升级完成");
        }
        self.changed();
        result
    }

    /// 文件换进托管目录、写记录；记录写不进去就换回去（账本写不进去时安装失败，ADR-0066 第 7 节）。
    async fn publish(
        &self,
        package: &PluginPackage,
        granted: &[Permission],
        old: Option<&InstalledRecord>,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let manifest = &package.manifest;
        let mut hosted = Vec::new();
        let swapped = match hosted_capability(manifest) {
            Some(capability) => {
                let root = package.static_root().unwrap_or_default();
                let files = package
                    .static_files()
                    .cloned()
                    .ok_or_else(|| InstallError::Invalid(Problem::Missing.message(&root, langs)))?;
                let dirs = self.hosted_dirs();
                let (id, digest) = (manifest.id.clone(), package.digest());
                let swapped = tokio::task::spawn_blocking(move || {
                    let staged = dirs.stage(capability, &id, &digest, &files)?;
                    dirs.swap_in(capability, &id, staged)
                })
                .await
                .map_err(|e| storage(langs, &e))?
                .map_err(|e| storage(langs, &e))?;
                hosted.push(capability.to_owned());
                Some(swapped)
            }
            None => None,
        };
        let created = match storage_need(manifest) {
            Some(StorageNeed::Required | StorageNeed::Wanted) => {
                self.prepare_storage(&manifest.id, langs)?
            }
            None => false,
        };
        // 数据跟着账本走：新版本不再要数据存储时，留着的数据照样记着，卸载时才知道要不要删。
        let had_storage = old.is_some_and(|old| old.hosted.iter().any(|c| c == WEB_STORAGE.name));
        if storage_need(manifest).is_some() || had_storage {
            hosted.push(WEB_STORAGE.name.to_owned());
        }
        let record = InstalledRecord {
            settings: Default::default(),
            settings_changed_ms: None,
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            source: package.source.clone(),
            files: package.files().clone(),
            granted_permissions: granted.to_vec(),
            enabled_permissions: Vec::new(),
            limits: Default::default(),
            lan_webui: false,
            relays: Vec::new(),
            firewalled: false,
            proxied: false,
            oidc_client_secret: None,
            installed_at: old.map_or_else(now_secs, |old| old.installed_at),
            origin: package.origin.clone(),
            entry_label: match old {
                Some(old) => old.entry_label.clone(),
                None => self.install_entry_label(&manifest.id)?,
            },
            written_by: old.and_then(|old| old.written_by.clone()),
            ask_every_update: old.is_some_and(|old| old.ask_every_update),
            family: old.is_some_and(|old| old.family),
            static_files: package.static_hashes().clone(),
            hosted,
        };
        let saved = self.store.save(&record);
        if saved.is_err()
            && created
            && let Err(e) = self.hosted_dirs().remove(WEB_STORAGE.name, &manifest.id)
        {
            tracing::warn!(plugin = %manifest.id, error = %e, "记录没存上，删不掉刚建的数据目录");
        }
        match (saved, swapped) {
            (Ok(()), swapped) => {
                if let Some(swapped) = swapped {
                    swapped.finish();
                }
                Ok(())
            }
            (Err(e), Some(swapped)) => {
                if let Err(undo) = swapped.undo() {
                    tracing::error!(plugin = %manifest.id, error = %undo, "记录没存上，换回旧的静态文件也失败");
                }
                Err(e.into())
            }
            (Err(e), None) => Err(e.into()),
        }
    }

    /// 核心启动时恢复不起容器的插件：静态网页的子目录不在就如实标异常（重装能修好），提供方的状态由
    /// [`effective_status`] 现看。
    pub(super) fn restore_containerless(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
        langs: &Languages,
    ) -> Entry {
        let listed = Listed::of_record(package, record);
        let missing = record
            .hosted
            .iter()
            .find(|capability| !self.hosted_dirs().exists(capability, &record.id));
        match missing {
            Some(capability) => {
                let key = if capability == WEB_STORAGE.name {
                    "plugin-hosted-data-missing"
                } else {
                    "plugin-hosted-dir-missing"
                };
                let reason = l10n::text(langs, key, &[("capability", capability.as_str().into())]);
                Entry::new(listed, PluginStatus::Degraded { reason }, None)
            }
            None => running(listed),
        }
    }

    /// 卸载时照记录（副作用账本）撤掉托管目录。数据存储的目录和容器插件的应用数据一样，`purge` 时才删。
    pub(super) fn remove_hosted(
        &self,
        id: &PluginId,
        purge: bool,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        // 记录读不出来（损坏的记录也要卸得掉）时没有账本可看：子目录按插件 ID 命名、只属于它，托管形态的
        // 都删，数据存储照 `purge`。
        let hosted = match self.store.load(id) {
            Ok(Some(record)) => record.hosted,
            Ok(None) => return Ok(()),
            Err(e) => {
                tracing::warn!(plugin = %id, error = %e, "读不出安装记录，按托管形态清理");
                capabilities::ALL
                    .iter()
                    .filter(|c| c.hosted.is_some_and(|h| purge || h.runtime.is_some()))
                    .map(|c| c.name.to_owned())
                    .collect()
            }
        };
        let dirs = self.hosted_dirs();
        for capability in hosted
            .iter()
            .filter(|capability| purge || *capability != WEB_STORAGE.name)
        {
            dirs.remove(capability, id)
                .map_err(|e| storage(langs, &e))?;
        }
        Ok(())
    }
}

fn running(listed: std::sync::Arc<Listed>) -> Entry {
    let mut entry = Entry::new(listed, PluginStatus::Running, None);
    entry.health = Some(Health::Healthy);
    entry
}

fn storage(langs: &Languages, error: &dyn std::fmt::Display) -> InstallError {
    storage_error(langs, error)
}

fn missing_provider(capability: &str, langs: &Languages) -> InstallError {
    InstallError::Conflict(l10n::text(
        langs,
        "plugin-hosted-provider-missing",
        &[("capability", capability.into())],
    ))
}
