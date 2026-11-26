//! 镜像与卷的操作：镜像的健康检查和平台列表，卷的建、查标签、删。

use std::collections::HashMap;

use bollard::models::{HealthConfig, VolumeCreateRequest};
use bollard::query_parameters::RemoveVolumeOptionsBuilder;

use super::{
    Engine, EngineError, ImagePlatforms, MEDIA_TYPE_INDEX, MEDIA_TYPE_LIST, Result, ignore_missing,
};

impl Engine {
    /// 镜像自带的 `HEALTHCHECK`；没有时为空。镜像要先拉下来（[`Self::ensure_image`]）。
    pub async fn image_healthcheck(&self, image: &str) -> Result<Option<HealthConfig>> {
        let inspect = self.docker.inspect_image(image).await.map_err(|e| {
            self.fail(
                "plugin-install-docker-context-image-healthcheck",
                &[("image", image.into())],
                e,
            )
        })?;
        Ok(inspect.config.and_then(|config| config.healthcheck))
    }

    /// 镜像的平台列表（`GET /distribution/{name}/json`，不拉镜像）：架构，以及单个清单时报的压缩后大小。
    /// 装了镜像仓库的凭据由 daemon 自己用。读不到时算查不出来，由调用方决定怎么显示。
    pub async fn image_platforms(&self, image: &str) -> Result<ImagePlatforms> {
        let inspect = self
            .docker
            .inspect_registry_image(image, None)
            .await
            .map_err(|e| {
                self.fail(
                    "plugin-install-docker-context-image-platforms",
                    &[("image", image.into())],
                    e,
                )
            })?;
        Ok(ImagePlatforms {
            architectures: inspect
                .platforms
                .into_iter()
                .filter_map(|platform| platform.architecture)
                .collect(),
            size: inspect
                .descriptor
                .media_type
                .as_deref()
                .is_some_and(|media_type| {
                    media_type != MEDIA_TYPE_INDEX && media_type != MEDIA_TYPE_LIST
                })
                .then_some(inspect.descriptor.size)
                .flatten()
                .and_then(|size| u64::try_from(size).ok()),
        })
    }

    pub async fn create_volume(&self, name: &str, labels: &HashMap<String, String>) -> Result<()> {
        let request = VolumeCreateRequest {
            name: Some(name.to_owned()),
            labels: Some(labels.clone()),
            ..Default::default()
        };
        self.docker
            .create_volume(request)
            .await
            .map(|_| ())
            .map_err(|e| self.fail_name("plugin-install-docker-context-create-volume", name, e))
    }

    /// 查询卷的标签；卷不存在时返回 `None`。
    pub async fn volume_labels(&self, name: &str) -> Result<Option<HashMap<String, String>>> {
        match self.docker.inspect_volume(name).await {
            Ok(volume) => Ok(Some(volume.labels)),
            Err(e) => match self.fail(
                "plugin-install-docker-context-volume-labels",
                &[("name", name.into())],
                e,
            ) {
                EngineError::NotFound(_) => Ok(None),
                other => Err(other),
            },
        }
    }

    pub async fn remove_volume(&self, name: &str) -> Result<()> {
        let options = RemoveVolumeOptionsBuilder::default().force(true).build();
        ignore_missing(
            self.patient()
                .remove_volume(name, Some(options))
                .await
                .map_err(|e| {
                    self.fail_name("plugin-install-docker-context-remove-volume", name, e)
                }),
        )
    }
}
