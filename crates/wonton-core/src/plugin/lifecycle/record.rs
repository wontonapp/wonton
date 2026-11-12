//! 安装成功后持久化主人确认的权限、设备选择和实际部署状态。

use super::super::manager::{InstallError, PluginManager, now_secs};
use super::super::manifest::Permission;
use super::super::package::PluginPackage;
use super::super::render::{Deployment, InstallContext};
use super::super::store::{EnabledPermission, InstalledDevice, InstalledRecord};

impl PluginManager {
    #[allow(clippy::too_many_arguments)] // 安装记录的各项来自已确认权限和实际部署结果。
    pub(super) fn save_installed(
        &self,
        package: &PluginPackage,
        granted: &[Permission],
        enabled: &[EnabledPermission],
        ctx: &InstallContext,
        plan: &Deployment,
        oidc_secret: Option<String>,
        firewalled: bool,
        devices: Vec<InstalledDevice>,
    ) -> Result<(), InstallError> {
        let manifest = &package.manifest;
        self.store.save(&InstalledRecord {
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            source: package.source.clone(),
            files: package.files().clone(),
            granted_permissions: granted.to_vec(),
            enabled_permissions: enabled.to_vec(),
            devices,
            limits: Default::default(),
            lan_webui: ctx.lan.as_ref().is_some_and(|lan| lan.webui),
            relays: plan.relays.clone(),
            firewalled,
            proxied: ctx.egress.is_some(),
            oidc_client_secret: oidc_secret,
            installed_at: now_secs(),
            entry_label: self.install_entry_label(&manifest.id)?,
            origin: package.origin.clone(),
            written_by: None,
            ask_every_update: false,
            family: false,
            settings: ctx.settings.clone().into_iter().collect(),
            settings_changed_ms: None,
            static_files: Default::default(),
            hosted: Vec::new(),
        })?;
        Ok(())
    }
}
