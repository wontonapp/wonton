//! 镜像加速（ADR-0040）。只改变按摘要拉取 Docker Hub 镜像的地址，不改宿主 Docker 配置。

pub(crate) mod address;
mod engine;
mod probe;
pub(crate) mod reference;

use std::sync::{Arc, RwLock};

use futures_util::future::join_all;
use wonton_protocol::{
    RegistryMirrorSettings, RegistryMirrorTest, RegistryMirrorTestRequest, RegistryMirrorTests,
    RegistryMirrorUpdate, RegistryProbe,
};

use crate::l10n;
use crate::plugin::docker::EngineError;
use crate::plugin::languages::Languages;
use crate::plugin::manager::{InstallError, PluginManager};
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
        let mirror = self.registry_mirror_address(langs)?;
        let (index, _, _) = self.storefront.official(false, langs).await;
        let docker_hub = self
            .registry_mirror
            .docker_hub
            .read()
            .expect("镜像测速锁中毒")
            .clone();
        let recommended = recommended(docker_hub.as_ref());
        Ok(RegistryMirrorSettings {
            mirror,
            choices: index.registry_mirrors,
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

    pub(in crate::plugin) fn registry_mirror_address(
        &self,
        langs: &Languages,
    ) -> Result<Option<String>, InstallError> {
        let mirror = self
            .store
            .registry_mirror()?
            .and_then(|settings| settings.mirror);
        if let Some(address) = &mirror {
            address::validate(address, langs).map_err(InstallError::Storage)?;
        }
        Ok(mirror)
    }

    pub(in crate::plugin) async fn prepare_images(
        &self,
        plan: &mut Deployment,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let mirror = self.registry_mirror_address(langs)?;
        for container in &mut plan.containers {
            let result = self
                .engine
                .ensure_image(&container.image, container.pull_policy, mirror.as_deref())
                .await;
            let image = result.map_err(|error| {
                let langs = self.owner_languages();
                self.registry_pull_error(error, mirror.as_deref(), &langs)
            })?;
            container.body.image = Some(image.clone());
            container.image = image;
        }
        Ok(())
    }

    pub(in crate::plugin) fn registry_pull_error(
        &self,
        error: EngineError,
        mirror: Option<&str>,
        langs: &Languages,
    ) -> InstallError {
        let suggest = mirror.is_none()
            && matches!(&error, EngineError::Pull { image, .. } if reference::docker_hub(image))
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
