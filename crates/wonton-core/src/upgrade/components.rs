//! 核心管理的其他组件（ADR-0025）：这些是后来的版本才有的，旧升级器叫不出名字，所以按标签认。切到一个镜像之前（升级、
//! 回滚和手动退回都一样），处理目标镜像不支持的组件。卷保留，再升回来时数据都在。

use super::LABEL_COMPONENTS;
use super::host::{Component, Host, HostError};

/// 处理 `image` 不支持的组件。这个镜像声明的以外都算不支持；没有这个标签等于一个都不支持。
pub async fn prepare(host: &dyn Host, image: &str) -> Result<(), HostError> {
    let info = host
        .image(image)
        .await?
        .ok_or_else(|| HostError::NotFound(format!("本机没有镜像 {image}")))?;
    let supported = info.labels.get(LABEL_COMPONENTS).map_or("", String::as_str);
    let supported: Vec<&str> = supported.split(',').map(str::trim).collect();
    for component in host.components().await? {
        match &component {
            Component::Project(project) if !supported.contains(&project.as_str()) => {
                tracing::info!(%project, %image, "目标镜像不支持这个组件，停掉它");
                host.compose_down(project).await?;
            }
            Component::Container {
                component,
                container,
            } if !supported.contains(&component.as_str()) => {
                tracing::info!(%component, %image, "目标镜像不支持这个组件，删掉它的容器");
                host.remove_container(container).await?;
            }
            _ => {}
        }
    }
    Ok(())
}
