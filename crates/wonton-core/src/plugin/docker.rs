//! Docker Engine API 的薄封装（bollard）。只暴露插件管理器用到的操作，并把错误归类：
//! Docker 不可达、资源不存在、Docker 拒绝请求。

use std::collections::HashMap;

use bollard::Docker;
use bollard::errors::Error as BollardError;
use bollard::models::{
    ContainerCreateBody, ContainerInspectResponse, EndpointSettings, NetworkConnectRequest,
    NetworkCreateRequest, NetworkDisconnectRequest, VolumeCreateRequest,
};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, ListContainersOptionsBuilder,
    ListNetworksOptionsBuilder, ListVolumesOptionsBuilder, LogsOptionsBuilder,
    RemoveContainerOptionsBuilder, RemoveVolumeOptionsBuilder, StopContainerOptionsBuilder,
    UploadToContainerOptionsBuilder,
};
use futures_util::StreamExt;

use super::compose::PullPolicy;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("无法连接 Docker：{0}")]
    Unavailable(String),
    #[error("Docker 中不存在：{0}")]
    NotFound(String),
    #[error("Docker 拒绝了请求（{context}）：{message}")]
    Rejected { context: String, message: String },
}

impl EngineError {
    fn from_bollard(context: &str, error: BollardError) -> Self {
        match error {
            BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            } => Self::NotFound(format!("{context}：{message}")),
            BollardError::DockerResponseServerError { message, .. }
            | BollardError::DockerStreamError { error: message } => Self::Rejected {
                context: context.to_owned(),
                message,
            },
            other => Self::Unavailable(format!("{context}：{other}")),
        }
    }
}

/// 按标签找到的属于某个插件的 Docker 资源。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LabeledResources {
    pub containers: Vec<String>,
    pub networks: Vec<String>,
    pub volumes: Vec<String>,
}

impl LabeledResources {
    pub fn is_empty(&self) -> bool {
        self.containers.is_empty() && self.networks.is_empty() && self.volumes.is_empty()
    }
}

#[derive(Clone)]
pub struct Engine {
    docker: Docker,
}

type Result<T> = std::result::Result<T, EngineError>;

impl Engine {
    /// 按 `DOCKER_HOST` 连接，未设置时使用本机 socket。
    pub fn connect() -> Result<Self> {
        let docker =
            Docker::connect_with_defaults().map_err(|e| EngineError::Unavailable(e.to_string()))?;
        Ok(Self { docker })
    }

    /// 测试用：连到给定地址上的 Engine API（测试里的假 Docker）。
    #[cfg(test)]
    pub(crate) fn http_for_test(url: &str) -> Self {
        let docker = Docker::connect_with_http(url, 5, bollard::API_DEFAULT_VERSION)
            .expect("构造测试用 Docker 客户端");
        Self { docker }
    }

    /// 测试用：指向一个不存在的 Docker，用于不需要 Docker 的网关测试。
    #[cfg(test)]
    pub(crate) fn unreachable_for_test() -> Self {
        let docker =
            Docker::connect_with_http("http://127.0.0.1:9", 1, bollard::API_DEFAULT_VERSION)
                .expect("构造测试用 Docker 客户端");
        Self { docker }
    }

    /// 检查 Docker 可达，返回宿主架构（如 `x86_64`、`aarch64`）。
    pub async fn host_arch(&self) -> Result<String> {
        let version = self
            .docker
            .version()
            .await
            .map_err(|e| EngineError::from_bollard("查询版本", e))?;
        Ok(version.arch.unwrap_or_default())
    }

    pub async fn ensure_image(&self, image: &str, policy: PullPolicy) -> Result<()> {
        let present = match self.docker.inspect_image(image).await {
            Ok(_) => true,
            Err(e) => match EngineError::from_bollard("检查镜像", e) {
                EngineError::NotFound(_) => false,
                other => return Err(other),
            },
        };
        match (policy, present) {
            (PullPolicy::Never, false) => Err(EngineError::NotFound(format!(
                "镜像 {image}（pull_policy: never）"
            ))),
            (PullPolicy::Never, true) | (PullPolicy::Missing, true) => Ok(()),
            _ => self.pull(image).await,
        }
    }

    async fn pull(&self, image: &str) -> Result<()> {
        let reference = pull_reference(image);
        let options = CreateImageOptionsBuilder::default()
            .from_image(&reference)
            .build();
        let mut stream = self.docker.create_image(Some(options), None, None);
        while let Some(item) = stream.next().await {
            item.map_err(|e| EngineError::from_bollard(&format!("拉取镜像 {image}"), e))?;
        }
        Ok(())
    }

    pub async fn create_network(&self, name: &str, labels: &HashMap<String, String>) -> Result<()> {
        let request = NetworkCreateRequest {
            name: name.to_owned(),
            driver: Some("bridge".to_owned()),
            labels: Some(labels.clone()),
            ..Default::default()
        };
        self.docker
            .create_network(request)
            .await
            .map(|_| ())
            .map_err(|e| EngineError::from_bollard(&format!("创建网络 {name}"), e))
    }

    /// 删除网络；不存在视为成功。
    pub async fn remove_network(&self, name: &str) -> Result<()> {
        ignore_missing(
            self.docker
                .remove_network(name)
                .await
                .map_err(|e| EngineError::from_bollard(&format!("删除网络 {name}"), e)),
        )
    }

    pub async fn connect_network(
        &self,
        network: &str,
        container: &str,
        aliases: Vec<String>,
    ) -> Result<()> {
        let request = NetworkConnectRequest {
            container: container.to_owned(),
            endpoint_config: Some(EndpointSettings {
                aliases: Some(aliases),
                ..Default::default()
            }),
        };
        self.docker
            .connect_network(network, request)
            .await
            .map_err(|e| {
                EngineError::from_bollard(&format!("把 {container} 接入网络 {network}"), e)
            })
    }

    /// 把容器移出网络；网络或连接不存在视为成功。
    pub async fn disconnect_network(&self, network: &str, container: &str) -> Result<()> {
        let request = NetworkDisconnectRequest {
            container: container.to_owned(),
            force: Some(true),
        };
        let result = self
            .docker
            .disconnect_network(network, request)
            .await
            .map_err(|e| {
                EngineError::from_bollard(&format!("把 {container} 移出网络 {network}"), e)
            });
        match result {
            Err(EngineError::Rejected { message, .. }) if message.contains("is not connected") => {
                Ok(())
            }
            other => ignore_missing(other),
        }
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
            .map_err(|e| EngineError::from_bollard(&format!("创建卷 {name}"), e))
    }

    /// 查询卷的标签；卷不存在时返回 `None`。
    pub async fn volume_labels(&self, name: &str) -> Result<Option<HashMap<String, String>>> {
        match self.docker.inspect_volume(name).await {
            Ok(volume) => Ok(Some(volume.labels)),
            Err(e) => match EngineError::from_bollard(&format!("查询卷 {name}"), e) {
                EngineError::NotFound(_) => Ok(None),
                other => Err(other),
            },
        }
    }

    pub async fn remove_volume(&self, name: &str) -> Result<()> {
        let options = RemoveVolumeOptionsBuilder::default().force(true).build();
        ignore_missing(
            self.docker
                .remove_volume(name, Some(options))
                .await
                .map_err(|e| EngineError::from_bollard(&format!("删除卷 {name}"), e)),
        )
    }

    pub async fn create_container(&self, name: &str, body: ContainerCreateBody) -> Result<()> {
        let options = CreateContainerOptionsBuilder::default().name(name).build();
        self.docker
            .create_container(Some(options), body)
            .await
            .map(|_| ())
            .map_err(|e| EngineError::from_bollard(&format!("创建容器 {name}"), e))
    }

    /// 把 tar 归档解到容器里的 `path` 下，容器创建后还没启动时也可以。属主和权限按归档里的记录；
    /// 要把已有的目录换成文件（或反过来）时报错，不覆盖。
    pub async fn upload_archive(
        &self,
        container: &str,
        path: &str,
        archive: Vec<u8>,
    ) -> Result<()> {
        let options = UploadToContainerOptionsBuilder::default()
            .path(path)
            .no_overwrite_dir_non_dir("true")
            .build();
        self.docker
            .upload_to_container(container, Some(options), bollard::body_full(archive.into()))
            .await
            .map_err(|e| EngineError::from_bollard(&format!("往容器 {container} 写入文件"), e))
    }

    pub async fn start_container(&self, name: &str) -> Result<()> {
        self.docker
            .start_container(name, None)
            .await
            .map_err(|e| EngineError::from_bollard(&format!("启动容器 {name}"), e))
    }

    /// 查询容器；不存在时返回 `None`。
    pub async fn inspect_container(&self, name: &str) -> Result<Option<ContainerInspectResponse>> {
        match self.docker.inspect_container(name, None).await {
            Ok(info) => Ok(Some(info)),
            Err(e) => match EngineError::from_bollard(&format!("查询容器 {name}"), e) {
                EngineError::NotFound(_) => Ok(None),
                other => Err(other),
            },
        }
    }

    /// 停止并删除容器（连同匿名卷）；不存在视为成功。
    pub async fn remove_container(&self, name: &str) -> Result<()> {
        let stop = StopContainerOptionsBuilder::default().t(10).build();
        let stopped = self
            .docker
            .stop_container(name, Some(stop))
            .await
            .map_err(|e| EngineError::from_bollard(&format!("停止容器 {name}"), e));
        match stopped {
            Ok(()) | Err(EngineError::NotFound(_)) | Err(EngineError::Rejected { .. }) => {}
            Err(other) => return Err(other),
        }
        let options = RemoveContainerOptionsBuilder::default()
            .force(true)
            .v(true)
            .build();
        ignore_missing(
            self.docker
                .remove_container(name, Some(options))
                .await
                .map_err(|e| EngineError::from_bollard(&format!("删除容器 {name}"), e)),
        )
    }

    /// 容器最后若干行日志，用于在安装失败时给出原因。
    pub async fn logs_tail(&self, name: &str, lines: usize) -> String {
        let options = LogsOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .tail(&lines.to_string())
            .build();
        let mut stream = self.docker.logs(name, Some(options));
        let mut out = String::new();
        while let Some(Ok(chunk)) = stream.next().await {
            out.push_str(&chunk.to_string());
        }
        out
    }

    /// 列出带 `key=value` 标签的容器、网络和卷（容器包括已停止的）。
    pub async fn labeled(&self, key: &str, value: &str) -> Result<LabeledResources> {
        let filters = HashMap::from([("label".to_owned(), vec![format!("{key}={value}")])]);
        let containers = self
            .docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default()
                    .all(true)
                    .filters(&filters)
                    .build(),
            ))
            .await
            .map_err(|e| EngineError::from_bollard("列出容器", e))?
            .into_iter()
            .filter_map(|c| {
                c.names
                    .and_then(|n| n.first().map(|n| n.trim_start_matches('/').to_owned()))
            })
            .collect();
        let networks = self
            .docker
            .list_networks(Some(
                ListNetworksOptionsBuilder::default()
                    .filters(&filters)
                    .build(),
            ))
            .await
            .map_err(|e| EngineError::from_bollard("列出网络", e))?
            .into_iter()
            .filter_map(|n| n.name)
            .collect();
        let volumes = self
            .docker
            .list_volumes(Some(
                ListVolumesOptionsBuilder::default()
                    .filters(&filters)
                    .build(),
            ))
            .await
            .map_err(|e| EngineError::from_bollard("列出卷", e))?
            .volumes
            .unwrap_or_default()
            .into_iter()
            .map(|v| v.name)
            .collect();
        Ok(LabeledResources {
            containers,
            networks,
            volumes,
        })
    }
}

/// 拉取用的镜像引用：没写标签也没写摘要时补上 `latest`，和 compose 一致。只传仓库名时
/// Engine API 会拉取该仓库的全部标签。标签只看最后一个 `/` 之后，仓库地址里的端口
/// （`localhost:5000/foo`）不算标签。
fn pull_reference(image: &str) -> String {
    let name = image.rsplit('/').next().unwrap_or(image);
    if image.contains('@') || name.contains(':') {
        image.to_owned()
    } else {
        format!("{image}:latest")
    }
}

fn ignore_missing(result: Result<()>) -> Result<()> {
    match result {
        Err(EngineError::NotFound(_)) => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::pull_reference;

    #[test]
    fn untagged_images_are_pulled_as_latest() {
        let cases = [
            ("nginx", "nginx:latest"),
            ("docker.io/library/redis", "docker.io/library/redis:latest"),
            ("localhost:5000/foo", "localhost:5000/foo:latest"),
            ("nginx:1.27", "nginx:1.27"),
            ("localhost:5000/foo:2", "localhost:5000/foo:2"),
            ("ghcr.io/org/app@sha256:abcd", "ghcr.io/org/app@sha256:abcd"),
            (
                "ghcr.io/org/app:1@sha256:abcd",
                "ghcr.io/org/app:1@sha256:abcd",
            ),
        ];
        for (image, expected) in cases {
            assert_eq!(pull_reference(image), expected, "{image}");
        }
    }
}
