//! 删除与重建：拆栈时按组件标签删容器；对账时认领同名容器、参数变了删了重建。

use std::collections::HashMap;

use super::create::Side;
use super::options::same_options;
use super::{CONNECTOR_COMPONENT, ENTRY_COMPONENT, EntryContainers, EntryError};
use crate::plugin::compose::PullPolicy;
use crate::plugin::names::{LABEL_COMPONENT, LABEL_MANAGED_BY};

impl EntryContainers {
    /// 拆栈和卸载时按组件标签删掉两个容器：它们挂着网络，compose `down` 删不掉。
    pub async fn remove_all(&self) -> Result<(), EntryError> {
        for component in [ENTRY_COMPONENT, CONNECTOR_COMPONENT] {
            let found = self
                .engine
                .labeled(LABEL_MANAGED_BY, "core", |labels| {
                    self.instance.owns(labels)
                        && labels.get(LABEL_COMPONENT).map(String::as_str) == Some(component)
                })
                .await?;
            for name in found.containers {
                self.engine.remove_container(&name).await?;
            }
            for network in found.networks {
                self.engine.remove_network(&network).await?;
            }
        }
        Ok(())
    }

    /// 创建一侧的容器：已经在了就只补齐网络；镜像或运行参数变了、或者是上一版 compose 留下的同名容器，删了重建；
    /// 名字被别的资源占着就停。
    /// 返回这次有没有建/重建容器（新容器起来时会自己读当前证书）。
    pub(super) async fn ensure(&self, side: &Side, project: &str) -> Result<bool, EntryError> {
        let name = self.name(side.component);
        let found = self.engine.inspect_container(&name).await?;
        let mut previous_version = false;
        if let Some(info) = &found {
            let labels = info
                .config
                .as_ref()
                .and_then(|config| config.labels.clone())
                .unwrap_or_default();
            if !self.owns(&labels, side.component, project) {
                return Err(EntryError::NameTaken(name));
            }
            previous_version = is_previous_version(&labels, side.component, project);
        }
        // 拉不到镜像时不动正在跑的容器。
        let image = self
            .engine
            .ensure_image(side.image, PullPolicy::Missing, &[])
            .await?;
        let current = found.as_ref().is_some_and(|info| {
            info.config
                .as_ref()
                .and_then(|config| config.image.as_deref())
                .is_some_and(|reference| {
                    crate::plugin::registry_mirror::reference::same_image(reference, side.image)
                })
        });
        let running = found
            .as_ref()
            .and_then(|info| info.state.as_ref())
            .and_then(|state| state.running)
            == Some(true);
        let mut body = side.body.clone();
        body.image = Some(image);
        let same_options = found.as_ref().is_some_and(|info| same_options(info, &body));
        let recreate = !current || !running || !same_options || previous_version;
        if found.is_some() && recreate {
            // 换法（复审 451 定）：先停删旧、再建新。旧容器在跑时占着 edge 的固定地址，Docker 在 start
            // 时才分配地址，并存必然 Address already in use（真 Docker 29.7.2 实测）——旧的不让位，新的
            // 起不来。接受换容器期间的短暂断开；fake Docker 有地址冲突模拟，顺序错了改端口测试会红。
            self.engine.remove_container(&name).await?;
            self.engine.create_container(&name, body).await?;
            for (network, endpoint) in &side.endpoints {
                self.engine
                    .connect_network_at(network, &name, endpoint.clone())
                    .await?;
            }
            self.engine.start_container(&name).await?;
            return Ok(true);
        }
        if recreate {
            self.engine.create_container(&name, body).await?;
        }
        for (network, endpoint) in &side.endpoints {
            if self.engine.endpoint(&name, network).await?.is_none() {
                self.engine
                    .connect_network_at(network, &name, endpoint.clone())
                    .await?;
            }
        }
        if recreate {
            self.engine.start_container(&name).await?;
        }
        Ok(recreate)
    }

    fn owns(&self, labels: &HashMap<String, String>, component: &str, project: &str) -> bool {
        self.instance.owns(labels)
            && (is_previous_version(labels, component, project)
                || labels.get(LABEL_MANAGED_BY).map(String::as_str) == Some("core")
                    && labels.get(LABEL_COMPONENT).map(String::as_str) == Some(component))
    }
}

/// 上一版 compose 建的同名容器：只带 compose 的标签、没有我们的组件标签。升级到这条线以后第一次启动时
/// 它们还占着名字，升级器要等 `up` 末尾的 `--remove-orphans` 才删，这里直接认领、删了重建。
fn is_previous_version(labels: &HashMap<String, String>, component: &str, project: &str) -> bool {
    labels.get("com.docker.compose.project").map(String::as_str) == Some(project)
        && labels.get("com.docker.compose.service").map(String::as_str) == Some(component)
        && labels.get(LABEL_MANAGED_BY).is_none()
}
