//! 恢复保留设备授权和缺失原因，只让缺设备的服务及其依赖者等待，不影响其余服务。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use bollard::models::ContainerStateStatusEnum;
use tokio::time::Instant;

use super::super::compose::DependCondition;
use super::super::health::{self, Target};
use super::super::languages::Languages;
use super::super::lifecycle::Undo;
use super::super::manager::{Deployed, InstallError, PluginManager};
use super::super::manifest::{Manifest, Permission, PluginId};
use super::super::names::LABEL_PLUGIN;
use super::super::package::PluginPackage;
use super::super::render::{Deployment, InstallContext, render, start_order};
use super::super::store::InstalledRecord;
use super::{DeviceGrant, grant, selection_from_record};
use crate::l10n::{list, text};

impl PluginManager {
    pub(in crate::plugin) fn recovery_devices(
        &self,
        manifest: &Manifest,
        record: &InstalledRecord,
    ) -> Result<DeviceGrant, InstallError> {
        let Some(devices) = &manifest.devices else {
            return Ok(DeviceGrant::default());
        };
        let langs = self.owner_languages();
        let granted = record.effective_permissions(manifest);
        let mut selection = selection_from_record(devices, &record.devices);
        if !granted.contains(&Permission::DevicesSerial) {
            selection.serial.clear();
        }
        if !granted.contains(&Permission::DevicesDisk) {
            selection.disks.clear();
        }
        for candidate in selection.serial.values_mut().chain(&mut selection.disks) {
            let path = visible(self.host.dev.as_deref(), &candidate.host_path);
            if let Some(metadata) = metadata(&path, &langs)? {
                use std::os::unix::fs::MetadataExt;
                candidate.gid = metadata.gid();
                let target =
                    fs::read_link(&path).map_err(|error| unreadable(&path, error, &langs))?;
                candidate.node = target
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| InstallError::DeviceUnavailable(path.display().to_string()))?
                    .to_owned();
            }
        }
        let udev = self.host.udev.as_deref().is_some_and(Path::is_dir);
        let mut result = grant(devices, &selection, &granted, udev);
        for (service, devices) in &result.services {
            for device in devices {
                if metadata(&visible(self.host.dev.as_deref(), &device.source), &langs)?.is_none() {
                    result
                        .unavailable
                        .insert(service.clone(), device.source.clone());
                }
            }
        }
        if granted.contains(&Permission::DevicesUsb)
            && metadata(&visible(self.host.dev.as_deref(), "/dev/bus/usb"), &langs)?.is_none()
        {
            for slot in &devices.usb {
                result
                    .unavailable
                    .insert(slot.service.clone(), "/dev/bus/usb".to_owned());
            }
        }
        Ok(result)
    }

    pub(in crate::plugin) async fn resume_device_services(
        &self,
        id: &PluginId,
        services: &[String],
        langs: &Languages,
    ) -> Result<Deployed, InstallError> {
        let record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::not_installed(langs, id))?;
        let mut package = PluginPackage::with_files(&record.source, record.files.clone(), langs)?;
        package.origin = record.origin.clone();
        let lan = self.lan_grant(&package, record.lan_webui, langs).await?;
        let mut ctx = self.rebuild_context(&package, &record, lan, None).await?;
        self.fill_bot_vars(&package, &mut ctx, langs).await?;
        let mut plan = render(&package, &ctx, langs)?;
        if let Some(devices) = &ctx.devices {
            devices.retain_available(&mut plan);
        }
        let mut affected = BTreeSet::new();
        plan.containers.retain(|container| {
            if !services.contains(&container.service)
                && !container
                    .depends_on
                    .iter()
                    .any(|(name, _)| affected.contains(name))
            {
                return false;
            }
            affected.insert(container.name.clone());
            true
        });
        let deadline = Instant::now() + self.cfg.install_timeout;
        for container in std::mem::take(&mut plan.containers) {
            let restart = services.contains(&container.service);
            plan.containers.push(container);
            self.resume_device_container(
                &package.manifest,
                &ctx,
                &mut plan,
                restart,
                deadline,
                langs,
            )
            .await?;
            plan.containers.clear();
        }
        self.reconcile_running(&package, &record).await
    }

    async fn resume_device_container(
        &self,
        manifest: &Manifest,
        ctx: &InstallContext,
        plan: &mut Deployment,
        restart: bool,
        deadline: Instant,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let container = &plan.containers[0];
        let Some(info) = self.engine.inspect_container(&container.name).await? else {
            self.prepare_images(plan, None, langs).await?;
            self.rewrite_image_healthchecks(ctx, plan).await?;
            let mut undo = Undo::default();
            self.prepare_dirs(ctx, &plan.dirs, &mut undo, langs)?;
            self.start_containers(Some(manifest), plan, None, &mut undo, deadline, langs)
                .await?;
            return Ok(());
        };
        let labels = info
            .config
            .and_then(|config| config.labels)
            .unwrap_or_default();
        if !self.cfg.instance.owns(&labels)
            || labels.get(LABEL_PLUGIN).map(String::as_str) != Some(manifest.id.as_str())
        {
            return Err(InstallError::Conflict(text(
                langs,
                "plugin-install-container-not-owned",
                &[
                    ("name", container.name.as_str().into()),
                    ("id", manifest.id.to_string().into()),
                ],
            )));
        }
        let state = info.state.unwrap_or_default();
        let completed = container.one_shot
            && state.status == Some(ContainerStateStatusEnum::EXITED)
            && state.exit_code == Some(0);
        if completed || (!restart && state.running == Some(true)) {
            return Ok(());
        }
        self.wait_dependencies(Some(manifest), &container.depends_on, deadline, langs)
            .await?;
        if restart {
            self.engine
                .restart_container(&container.name, langs)
                .await?;
        } else {
            self.engine.start_container(&container.name).await?;
        }
        let target = if container.one_shot {
            Target::Depend(DependCondition::ServiceCompletedSuccessfully)
        } else {
            Target::Ready
        };
        let webui = self.webui_health(manifest, &container.service).await?;
        health::wait_for(
            &self.engine,
            &self.http,
            &container.name,
            target,
            webui.as_ref(),
            deadline,
            langs,
        )
        .await?;
        Ok(())
    }
}

impl DeviceGrant {
    pub(in crate::plugin) fn problem(&self, langs: &Languages) -> Option<String> {
        if self.unavailable.is_empty() {
            return None;
        }
        let names = self
            .unavailable
            .values()
            .map(String::as_str)
            .collect::<Vec<_>>();
        Some(text(
            langs,
            "plugin-install-device-gone",
            &[("devices", list(langs, &names).into())],
        ))
    }

    pub(in crate::plugin) fn blocked_services(
        &self,
        package: &PluginPackage,
        langs: &Languages,
    ) -> Result<BTreeSet<String>, InstallError> {
        let mut blocked = self.unavailable.keys().cloned().collect::<BTreeSet<_>>();
        if blocked.is_empty() {
            return Ok(blocked);
        }
        let dependencies = package.service_dependencies(langs)?;
        let order = start_order(
            dependencies
                .iter()
                .map(|(name, deps)| (*name, deps.iter().map(|(name, _)| name.clone()).collect()))
                .collect(),
            langs,
        )?;
        for service in order {
            if dependencies[service.as_str()]
                .iter()
                .any(|(dependency, _)| blocked.contains(dependency))
            {
                blocked.insert(service);
            }
        }
        Ok(blocked)
    }

    pub(in crate::plugin) fn retain_available(&self, plan: &mut Deployment) {
        if self.unavailable.is_empty() {
            return;
        }
        let mut blocked = BTreeSet::new();
        plan.containers.retain(|container| {
            let missing = self.unavailable.contains_key(&container.service)
                || container
                    .depends_on
                    .iter()
                    .any(|(name, _)| blocked.contains(name));
            if missing {
                blocked.insert(container.name.clone());
            }
            !missing
        });
    }
}

fn visible(dev: Option<&Path>, source: &str) -> PathBuf {
    dev.map_or_else(
        || PathBuf::from(source),
        |dev| dev.join(source.trim_start_matches("/dev/")),
    )
}

fn metadata(path: &Path, langs: &Languages) -> Result<Option<fs::Metadata>, InstallError> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(unreadable(path, error, langs)),
    }
}

fn unreadable(path: &Path, error: std::io::Error, langs: &Languages) -> InstallError {
    InstallError::DeviceUnavailable(text(
        langs,
        "plugin-install-device-unreadable",
        &[
            ("path", path.display().to_string().into()),
            ("error", error.to_string().into()),
        ],
    ))
}
