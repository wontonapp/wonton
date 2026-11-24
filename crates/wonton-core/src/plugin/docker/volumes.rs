//! 卷：建、查标签、删，以及镜像声明的匿名卷目标。

use std::collections::HashMap;

use bollard::models::VolumeCreateRequest;
use bollard::query_parameters::RemoveVolumeOptionsBuilder;

use super::{Engine, EngineError, Result, ignore_missing};

impl Engine {
    /// 镜像声明的匿名卷目标（`VOLUME`）：换容器时这些目标也要接回原来那份卷。
    pub async fn image_volumes(&self, image: &str) -> Result<Vec<String>> {
        let inspect = self.docker.inspect_image(image).await.map_err(|e| {
            self.fail(
                "plugin-install-docker-context-image-volumes",
                &[("image", image.into())],
                e,
            )
        })?;
        Ok(inspect
            .config
            .and_then(|config| config.volumes)
            .unwrap_or_default())
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
