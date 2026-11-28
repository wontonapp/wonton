//! 执行升级：保留数据、插件密钥、Bot 账号和设备、模型凭据、OIDC 客户端，只换容器。
//!
//! 先把能失败的都做在旧容器还在跑的时候（渲染、拉镜像、建目录和卷），这些失败了旧版本不受影响。换容器之后失败就退回：用旧记录和旧容器当时的镜像重建旧版本。Bot 的令牌从旧容器读回（见 [`crate::plugin::egress`]），只留在内存里和容器的环境
//! 变量里，这里绝不签新设备：换设备要重新建加密身份。安装记录最后才写，核心中途退出时记录还是旧版本的。

use std::collections::HashMap;
use std::sync::Arc;

use super::super::compose::PullPolicy;
use super::super::lan::webui::switched;
use super::super::languages::Languages;
use super::super::lifecycle::{Grants, Undo};
use super::super::listed::Listed;
use super::super::manager::{
    Deployed, Entry, Health, InstallError, PluginManager, PluginStatus, busy, check_permissions,
    now_secs,
};
use super::super::manifest::{ManifestExt, Permission, PluginId, PluginKind};
use super::super::package::PluginPackage;
use super::super::proxy_auth::ProxyAuth;
use super::super::render::{Deployment, InstallContext, dry_run};
use super::super::secrets::Secrets;
use super::super::store::{EnabledPermission, InstalledRecord};
use super::newer;
use crate::l10n;

fn old_limits(entry: &Entry) -> wonton_protocol::PluginLimits {
    entry
        .listed
        .as_ref()
        .map(|listed| listed.limits)
        .unwrap_or_default()
}

/// 退回旧版本要用的：Bot 的令牌在 `ctx` 里。
pub(super) struct Old {
    pub(super) record: InstalledRecord,
    pub(super) package: PluginPackage,
    pub(super) ctx: InstallContext,
    /// 服务 → 旧容器当时的镜像：标签可能已经被新版本的拉取移动了。
    pub(super) images: HashMap<String, String>,
}

/// 做到哪了，失败时据此收拾。
#[derive(Default)]
pub(super) struct Progress {
    pub(super) undo: Undo,
    /// 旧容器动过了：失败时要重建旧版本。
    pub(super) touched: bool,
    pub(super) network_rebuilt: bool,
    /// 这次多签发的模型网关凭据。
    pub(super) added_models: Vec<String>,
}

/// 新版本部署要用的，都在旧容器还在跑的时候备好。
pub(super) struct Prepared {
    pub(super) ctx: InstallContext,
    pub(super) devices: Vec<super::super::store::InstalledDevice>,
    pub(super) secrets: Secrets,
    pub(super) proxy: Option<ProxyAuth>,
    pub(super) oidc_secret: Option<String>,
    pub(super) pulled: Deployment,
}

enum Outcome {
    Upgraded(Deployed),
    /// 旧容器没动过：状态放回去。
    Untouched(InstallError),
    /// 换过容器，退回了旧版本。
    RolledBack(Arc<Listed>, Deployed, InstallError),
    RollbackFailed(Arc<Listed>, InstallError),
}

/// 旧容器之前的样子，旧容器没动过时放回去。
struct Before {
    status: PluginStatus,
    health: Option<Health>,
}

pub(super) fn pin_images(plan: &mut Deployment, images: &HashMap<String, String>) {
    for container in &mut plan.containers {
        if let Some(image) = images.get(&container.service) {
            container.image.clone_from(image);
            container.body.image = Some(image.clone());
            container.pull_policy = PullPolicy::Missing;
        }
    }
}

impl PluginManager {
    /// 检查并升级。`granted` 必须含新版本的全部必需权限：这就是主人的确认；`switches` 是主人勾选的可选项，
    /// 或者沿用记录里的开关。
    pub(super) async fn upgrade_declared(
        &self,
        package: PluginPackage,
        granted: &[Permission],
        switches: &super::UpgradeSwitches,
        lan_webui: bool,
        chosen_devices: &[String],
        settings: super::super::settings::Values,
    ) -> Result<(), InstallError> {
        let langs = self.owner_languages();
        super::super::install_entry::check_runtime(&package.manifest, &langs)?;
        self.check_not_upgrading(&langs)?;
        if !super::super::install_hosted::is_containerless(&package.manifest) {
            dry_run(&package, &langs)?;
        }
        self.check_icon(&package, &langs).await?;
        let manifest = &package.manifest;
        let id = &manifest.id;
        // 生效的可选项按这一刻读到的记录算：沿用开关的升级在读和用之间不留空档（#767）。
        let record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::NotInstalled(id.to_string()))?;
        if record.lifecycle.desired == wonton_protocol::PluginDesiredState::Stopped {
            return Err(InstallError::Conflict(l10n::text(
                &langs,
                "plugin-recovery-owner-stopped",
                &[],
            )));
        }
        let old = super::installed_package(&record, &langs)?;
        let enabled = super::switched_on(manifest, &record, &old.manifest, switches);
        check_permissions(manifest, granted, &enabled, &langs)?;
        super::super::capabilities::unsupported_model_dependency(manifest, &langs)?;
        if super::super::install_hosted::is_containerless(manifest) {
            return self.upgrade_containerless(package, granted, &langs).await;
        }
        self.check_hosted_provider(manifest, &langs)?;
        self.check_requirements(manifest, &langs).await?;
        self.check_egress(manifest, &langs).await?;
        let effective =
            manifest.effective_permissions(&super::super::store::permission_set(&enabled));
        let gpu = self.gpu_grant(manifest, &effective, &langs).await?;
        let lan_webui = lan_webui && switched(&package).is_some();
        let lan = self.lan_grant(&package, lan_webui, &langs).await?;
        let (devices, device_record) = self.upgrade_devices(
            manifest,
            &effective,
            &record.devices,
            chosen_devices,
            &langs,
        )?;
        let before = self.begin_upgrade(id)?;
        tracing::info!(plugin = %id, version = %manifest.version, "开始升级插件");
        // 放堆上跑：从建容器到对账的一整串都在这条链里，调试构建的帧很大，嵌套两层就够顶满测试线程的 2 MiB 栈。
        let outcome = Box::pin(self.upgrade_begun(
            &package,
            granted,
            &enabled,
            Grants {
                gpu,
                lan,
                devices,
                device_record,
            },
            settings.clone(),
            &langs,
        ))
        .await;
        self.lan.end(id);
        let result = self.finish_upgrade(&package, lan_webui, before, outcome);
        if result.is_ok()
            && super::super::egress::provider::provides_proxy(manifest)
            && self.egress_config()?.source == wonton_protocol::EgressProxySource::Plugin
        {
            self.follow_core_proxy();
            self.reconcile_relay().await?;
        }
        self.changed();
        result
    }

    /// 和卸载一样在插件表的锁里判断：在安装、卸载、升级或者按新地址重建的不碰。
    fn begin_upgrade(&self, id: &PluginId) -> Result<Before, InstallError> {
        let langs = self.owner_languages();
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let entry = plugins
            .get_mut(id)
            .ok_or_else(|| InstallError::not_installed(&langs, id))?;
        if let Some(e) = busy(id, &entry.status, &langs) {
            return Err(e);
        }
        if self.explicitly_stopped(id)? {
            return Err(InstallError::Conflict(l10n::text(
                &langs,
                "plugin-recovery-owner-stopped",
                &[],
            )));
        }
        if !self.lan.begin(id) {
            return Err(InstallError::Conflict(l10n::text(
                &langs,
                "plugin-install-conflict-lan-rebuilding",
                &[("id", id.to_string().into())],
            )));
        }
        let before = Before {
            status: entry.status.clone(),
            health: entry.health,
        };
        entry.status = PluginStatus::Upgrading;
        entry.install_progress = None;
        drop(plugins);
        self.changed();
        Ok(before)
    }

    fn finish_upgrade(
        &self,
        package: &PluginPackage,
        lan_webui: bool,
        before: Before,
        outcome: Outcome,
    ) -> Result<(), InstallError> {
        let id = &package.manifest.id;
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let Some(entry) = plugins.get_mut(id) else {
            let langs = self.owner_languages();
            return Err(InstallError::not_installed(&langs, id));
        };
        entry.install_progress = None;
        match outcome {
            Outcome::Upgraded(deployed) => {
                let installed_at = entry
                    .listed
                    .as_ref()
                    .map_or_else(now_secs, |l| l.installed_at);
                let listed = Listed::new(
                    package,
                    installed_at,
                    lan_webui,
                    deployed.firewalled,
                    old_limits(entry),
                );
                *entry = Entry::running(listed, deployed);
                self.lan.clear_degraded(id);
                self.forget_bot_sync(id);
                self.upgrades.forget(id);
                tracing::info!(plugin = %id, version = %package.manifest.version, "插件升级完成");
                Ok(())
            }
            Outcome::Untouched(e) => {
                entry.status = before.status;
                entry.health = before.health;
                tracing::warn!(plugin = %id, error = %e, "插件升级失败，旧版本没动");
                Err(e)
            }
            Outcome::RolledBack(listed, deployed, e) => {
                *entry = Entry::running(listed, deployed);
                self.forget_bot_sync(id);
                tracing::warn!(plugin = %id, error = %e, "插件升级失败，已退回旧版本");
                Err(e)
            }
            Outcome::RollbackFailed(listed, e) => {
                let reason = e.to_string();
                *entry = Entry::new(listed, PluginStatus::Degraded { reason }, None);
                tracing::error!(plugin = %id, error = %e, "插件升级失败，退回旧版本也失败");
                Err(e)
            }
        }
    }

    async fn upgrade_begun(
        &self,
        package: &PluginPackage,
        granted: &[Permission],
        enabled: &[EnabledPermission],
        grants: Grants,
        settings: super::super::settings::Values,
        langs: &Languages,
    ) -> Outcome {
        let old = match self.read_old(&package.manifest.id).await {
            Ok(old) => old,
            Err(e) => return Outcome::Untouched(e),
        };
        let mut progress = Progress::default();
        let error = match self
            .replace_version(
                &old,
                package,
                granted,
                enabled,
                grants,
                settings,
                &mut progress,
            )
            .await
        {
            Ok(deployed) => {
                self.drop_unused(&old, package, langs).await;
                return Outcome::Upgraded(deployed);
            }
            Err(e) => e,
        };
        let version = &old.record.version;
        let new = &package.manifest.version;
        if !progress.touched {
            return Outcome::Untouched(self.undo_prepared(&old, progress, error).await);
        }
        let listed = Listed::of_record(&old.package, &old.record);
        match self.back_to_old(&old, package, progress).await {
            Ok(deployed) => {
                let note = l10n::text(
                    langs,
                    "plugin-settings-upgrade-rolled-back",
                    &[
                        ("new", new.as_str().into()),
                        ("version", version.as_str().into()),
                    ],
                );
                Outcome::RolledBack(listed, deployed, error.with_note(langs, &note))
            }
            Err(back) => {
                let note = l10n::text(
                    langs,
                    "plugin-settings-upgrade-rollback-failed",
                    &[
                        ("new", new.as_str().into()),
                        ("version", version.as_str().into()),
                        ("error", back.to_string().into()),
                    ],
                );
                Outcome::RollbackFailed(listed, error.with_note(langs, &note))
            }
        }
    }

    /// 系统插件随核心升级（插件规范的“预装插件”）：目录里的版本比装着的新时按同样的步骤升级，装核心就是对它的确认。启动时
    /// 等核心自己的升级结束才走到这里（见 `main.rs` 的预装）。失败时旧版本继续跑，如实列出，下次启动再试。
    pub(in crate::plugin) async fn upgrade_bundled(&self, package: PluginPackage) {
        let manifest = &package.manifest;
        let id = manifest.id.clone();
        if manifest.kind != PluginKind::System {
            return;
        }
        let installed = match self.store.load(&id) {
            Ok(Some(record)) => record,
            Ok(None) => return,
            Err(e) => {
                tracing::error!(plugin = %id, error = %e, "读系统插件的记录失败，没有升级");
                return;
            }
        };
        let version = manifest.version.clone();
        if !newer(&version, &installed.version) {
            return;
        }
        let granted = manifest.permissions.clone();
        // 和 install_checked 一样：upgrade 子树的状态机搬上堆，默认 2 MiB 测试线程栈装不下整条 await 链。
        if let Err(e) = Box::pin(self.upgrade_checked(
            package,
            &granted,
            &super::UpgradeSwitches::Requested(installed.enabled_permissions.clone()),
            installed.lan_webui,
            &[],
            &[],
            installed.settings.clone(),
        ))
        .await
        {
            let from = installed.version;
            tracing::error!(plugin = %id, error = %e, "系统插件升级失败");
            let reason = l10n::text(
                &self.owner_languages(),
                "plugin-settings-upgrade-bundled-failed",
                &[
                    ("version", version.as_str().into()),
                    ("from", from.as_str().into()),
                    ("error", e.to_string().into()),
                ],
            );
            self.upgrades.bundled_failed(&id, reason);
            self.changed();
        }
    }
}
