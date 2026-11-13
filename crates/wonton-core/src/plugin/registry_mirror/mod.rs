//! 镜像加速（ADR-0074，按仓库主机分名单）。只改变按摘要拉取镜像的地址，不改宿主 Docker 配置。

pub(crate) mod address;
mod engine;
pub(in crate::plugin) mod mirrors;
mod probe;
pub(crate) mod reference;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use futures_util::future::join_all;
use wonton_protocol::{
    InstallProgress, InstallStage, RegistryMirrorSettings, RegistryMirrorTest,
    RegistryMirrorTestRequest, RegistryMirrorTests, RegistryMirrorUpdate, RegistryProbe,
};

use crate::l10n;
use crate::plugin::docker::EngineError;
use crate::plugin::languages::Languages;
use crate::plugin::manager::{InstallError, PluginManager};
use crate::plugin::manifest::PluginId;
pub(in crate::plugin) use engine::LayerProgress;
use mirrors::Mirrors;

/// 一张镜像按层累加的下载进度。Docker 每条只带一层的字节数，层还会交错，所以按层的 ID 记住、把它们加起来。
#[derive(Default)]
pub(in crate::plugin) struct LayerBytes {
    layers: HashMap<String, (u64, u64, bool)>,
    last: u8,
}

impl LayerBytes {
    /// 记一层的一条进展，返回新的百分比。层还没报过总数时是 0；看到的层都完成了就是 100。只增不减。
    pub(in crate::plugin) fn update(&mut self, layer: LayerProgress<'_>) -> u8 {
        let entry = self
            .layers
            .entry(layer.id.to_owned())
            .or_insert((0, 0, false));
        entry.1 = entry.1.max(layer.total);
        entry.0 = entry.0.max(layer.current).min(entry.1.max(layer.current));
        entry.2 |= layer.done;
        // 完成的事件（`Pull complete` 这类）没有字节数：按这一层满了算。
        if layer.done {
            entry.0 = entry.1;
        }
        let known: u64 = self.layers.values().map(|(_, total, _)| *total).sum();
        let current: u64 = self.layers.values().map(|(current, _, _)| *current).sum();
        let all_done = !self.layers.is_empty() && self.layers.values().all(|(_, _, done)| *done);
        let percent = if all_done {
            100
        } else {
            current
                .saturating_mul(100)
                .checked_div(known)
                .unwrap_or_default()
                .min(99)
        };
        self.last = (percent.min(100) as u8).max(self.last);
        self.last
    }
}
use crate::plugin::render::Deployment;

#[derive(Default)]
pub(super) struct RegistryMirror {
    docker_hub: RwLock<Option<RegistryProbe>>,
    probing: tokio::sync::Mutex<()>,
}

impl PluginManager {
    pub async fn registry_mirror_settings(
        &self,
        langs: &Languages,
    ) -> Result<RegistryMirrorSettings, InstallError> {
        let mirror = self
            .store
            .registry_mirror()?
            .and_then(|settings| settings.mirror);
        if let Some(address) = &mirror {
            address::validate(address, langs).map_err(InstallError::Storage)?;
        }
        let (index, _, problem) = self.storefront.official(false, langs).await;
        let choices_problem = index
            .registry_mirrors_problem
            .or_else(|| problem.map(|problem| problem.details));
        let docker_hub = self
            .registry_mirror
            .docker_hub
            .read()
            .expect("镜像测速锁中毒")
            .clone();
        let recommended = recommended(docker_hub.as_ref());
        Ok(RegistryMirrorSettings {
            mirror,
            choices: index.registry_mirrors.docker_hub,
            choices_problem,
            docker_hub,
            recommended,
        })
    }

    pub async fn set_registry_mirror(
        &self,
        update: RegistryMirrorUpdate,
        langs: &Languages,
    ) -> Result<RegistryMirrorSettings, InstallError> {
        if let Some(address) = &update.mirror {
            address::validate(address, langs).map_err(InstallError::Invalid)?;
        }
        self.store.save_registry_mirror(&update)?;
        self.registry_mirror_settings(langs).await
    }

    pub async fn test_registry_mirrors(
        &self,
        request: RegistryMirrorTestRequest,
        langs: &Languages,
    ) -> Result<RegistryMirrorTests, InstallError> {
        for address in &request.mirrors {
            address::validate(address, langs).map_err(InstallError::Invalid)?;
        }
        let direct = self.probe_registry_hub(langs);
        let mirrors = {
            let langs = langs.clone();
            join_all(request.mirrors.into_iter().map(move |mirror| {
                let langs = langs.clone();
                async move {
                    let probe = probe::probe(&mirror, &langs).await;
                    RegistryMirrorTest { mirror, probe }
                }
            }))
        };
        let (docker_hub, mirrors) = tokio::join!(direct, mirrors);
        Ok(RegistryMirrorTests {
            docker_hub: docker_hub?,
            mirrors,
        })
    }

    async fn probe_registry_hub(
        &self,
        langs: &Languages,
    ) -> Result<Option<RegistryProbe>, InstallError> {
        let _pass = self.registry_mirror.probing.lock().await;
        let probe = if self.engine.registry_proxy_configured().await? {
            None
        } else {
            Some(probe::probe("registry-1.docker.io", langs).await)
        };
        *self
            .registry_mirror
            .docker_hub
            .write()
            .expect("镜像测速锁中毒") = probe.clone();
        Ok(probe)
    }

    pub fn spawn_registry_mirror_probe(self: &Arc<Self>) {
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(error) = manager.probe_registry_hub(&manager.owner_languages()).await {
                tracing::warn!(%error, "Docker Hub 直连测速失败");
            }
        });
    }

    /// 这张机器能用的加速名单，加速关掉时为空。打开时主人填的 Docker Hub 地址排在最前面，其余按索引的顺序（ADR-0074）。
    pub(in crate::plugin) async fn registry_mirrors(
        &self,
        langs: &Languages,
    ) -> Result<Mirrors, InstallError> {
        let Some(address) = self
            .store
            .registry_mirror()?
            .and_then(|settings| settings.mirror)
        else {
            return Ok(Mirrors::default());
        };
        address::validate(&address, langs).map_err(InstallError::Storage)?;
        let mut mirrors = self
            .storefront
            .official(false, langs)
            .await
            .0
            .registry_mirrors;
        mirrors.docker_hub.insert(0, address);
        mirrors.dedupe();
        Ok(mirrors)
    }

    pub(in crate::plugin) async fn prepare_images(
        &self,
        plan: &mut Deployment,
        id: Option<&PluginId>,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        if let Some(id) = id {
            self.stage(id, InstallStage::PullingImages);
        }
        let mirrors = self.registry_mirrors(langs).await?;
        let count = plan.containers.len().max(1) as u64;
        for (index, container) in plan.containers.iter_mut().enumerate() {
            let mut bytes = LayerBytes::default();
            let mut report = |layer: LayerProgress<'_>| {
                let Some(id) = id else { return };
                let done = u64::from(bytes.update(layer));
                let percent = ((index as u64 * 100 + done) / count).min(100) as u8;
                self.report_progress(
                    id,
                    InstallProgress {
                        stage: InstallStage::PullingImages,
                        percent: Some(percent),
                    },
                );
            };
            let accelerated = accelerated(&container.image, &mirrors);
            let result = self
                .engine
                .ensure_image_with_progress(
                    &container.image,
                    container.pull_policy,
                    &accelerated,
                    &mut report,
                )
                .await;
            let image = result.map_err(|error| {
                let langs = self.owner_languages();
                self.registry_pull_error(error, mirrors.docker_hub.is_empty(), &langs)
            })?;
            container.body.image = Some(image.clone());
            container.image = image;
        }
        Ok(())
    }

    pub(in crate::plugin) fn registry_pull_error(
        &self,
        error: EngineError,
        acceleration_off: bool,
        langs: &Languages,
    ) -> InstallError {
        let suggest = acceleration_off
            && matches!(&error, EngineError::Pull { image, .. } if mirrors::registry(image).is_some())
            && recommended(
                self.registry_mirror
                    .docker_hub
                    .read()
                    .expect("镜像测速锁中毒")
                    .as_ref(),
            );
        let error = InstallError::from(error);
        if suggest {
            error.with_note(
                langs,
                &l10n::text(langs, "plugin-settings-note-registry-mirror", &[]),
            )
        } else {
            error
        }
    }
}

/// 这张镜像的加速引用：仓库有名单、引用按摘要钉住时才改。
pub(in crate::plugin) fn accelerated(image: &str, mirrors: &Mirrors) -> Vec<String> {
    match mirrors::registry(image) {
        Some(registry) => reference::accelerated(image, mirrors.stations(registry), registry),
        None => Vec::new(),
    }
}

fn recommended(probe: Option<&RegistryProbe>) -> bool {
    probe.is_some_and(|probe| {
        probe.problem.is_some() || probe.latency_ms.is_some_and(|ms| ms > 5_000)
    })
}

pub(in crate::plugin) fn use_pulled_images(plan: &mut Deployment, pulled: Deployment) {
    // 同一个插件包只换网络上下文，render 的服务集合和顺序不变。
    debug_assert_eq!(plan.containers.len(), pulled.containers.len());
    for (container, pulled) in plan.containers.iter_mut().zip(pulled.containers) {
        debug_assert_eq!(container.service, pulled.service);
        container.image = pulled.image;
        container.body.image = pulled.body.image;
    }
}

#[cfg(test)]
mod docker_fixture;
#[cfg(test)]
mod docker_tests;
#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
