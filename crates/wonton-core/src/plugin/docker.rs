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
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, DownloadFromContainerOptionsBuilder,
    ListContainersOptionsBuilder, ListNetworksOptionsBuilder, ListVolumesOptionsBuilder,
    LogsOptionsBuilder, RemoveContainerOptionsBuilder, RemoveVolumeOptionsBuilder,
    StopContainerOptionsBuilder, UploadToContainerOptionsBuilder,
};
use futures_util::StreamExt;

use super::compose::PullPolicy;
use super::sources::Subnet;

/// 核心要求的 Docker Engine 最低主版本：28 起局域网里的其他主机不能再经路由直接访问容器里没发布的端口
/// （moby#48724），内部网络也才有 `isolated` 网关模式。
pub const MIN_ENGINE_MAJOR: u32 = 28;

const GATEWAY_MODES: [&str; 2] = [
    "com.docker.network.bridge.gateway_mode_ipv4",
    "com.docker.network.bridge.gateway_mode_ipv6",
];

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("无法连接 Docker：{0}")]
    Unavailable(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("Docker 中不存在：{0}")]
    NotFound(String),
    #[error("Docker 拒绝了请求（{context}）：{message}")]
    Rejected { context: String, message: String },
}

impl EngineError {
    pub(super) fn from_bollard(context: &str, error: BollardError) -> Self {
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

/// Docker 守护进程的版本和宿主架构。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    pub version: String,
    pub arch: String,
}

/// 插件网络能连到哪里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkReach {
    /// 普通桥接网络：能访问外网和局域网。
    Egress,
    /// 内部网络：没有到外面的路由，也不转发外部 DNS；宿主经网桥还能和容器互访（核心直接跑在宿主上时）。
    Internal,
    /// 内部网络，网桥在宿主上也没有地址：容器只能和同一网络里的容器（包括核心）互访，宿主上的服务也访问不到。
    Isolated,
}

/// 读容器里一个文件的结果。
#[derive(Debug, PartialEq, Eq)]
pub enum ContainerFile {
    Missing,
    /// 是目录、软链接、设备等，不是普通文件。
    NotRegular,
    TooBig,
    Regular(Vec<u8>),
}

/// 归档除了文件内容之外的开销（头部、PAX 扩展、结尾的空块）的上限。
const ARCHIVE_SLACK: usize = 16 << 10;

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
    pub(super) docker: Docker,
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
        Ok(self.server().await?.arch)
    }

    async fn server(&self) -> Result<ServerInfo> {
        let version = self
            .docker
            .version()
            .await
            .map_err(|e| EngineError::from_bollard("查询版本", e))?;
        Ok(ServerInfo {
            version: version.version.unwrap_or_default(),
            arch: version.arch.unwrap_or_default(),
        })
    }

    /// Docker 登记的 OCI 运行时的名字（`runc`，装了 NVIDIA Container Toolkit 时还有 `nvidia`）。
    pub async fn runtimes(&self) -> Result<Vec<String>> {
        let info = self
            .docker
            .info()
            .await
            .map_err(|e| EngineError::from_bollard("查询 Docker 信息", e))?;
        let mut runtimes: Vec<String> = info.runtimes.unwrap_or_default().into_keys().collect();
        runtimes.sort();
        Ok(runtimes)
    }

    /// 核心启动时检查 Docker：连得上，而且 Engine 版本不低于 [`MIN_ENGINE_MAJOR`]。低于时报错，不降级运行：
    /// 插件网络的隔离靠的是新版的行为。
    pub async fn check_server(&self) -> Result<ServerInfo> {
        let info = self.server().await?;
        check_engine_version(&info.version)?;
        Ok(info)
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

    pub async fn create_network(
        &self,
        name: &str,
        labels: &HashMap<String, String>,
        reach: NetworkReach,
    ) -> Result<()> {
        let isolated = GATEWAY_MODES.map(|mode| (mode.to_owned(), "isolated".to_owned()));
        let request = NetworkCreateRequest {
            name: name.to_owned(),
            driver: Some("bridge".to_owned()),
            labels: Some(labels.clone()),
            internal: Some(reach != NetworkReach::Egress),
            options: (reach == NetworkReach::Isolated).then(|| isolated.into()),
            ..Default::default()
        };
        self.docker
            .create_network(request)
            .await
            .map(|_| ())
            .map_err(|e| EngineError::from_bollard(&format!("创建网络 {name}"), e))
    }

    /// 网络能连到哪里；网络不存在时返回 `None`。
    pub async fn network_reach(&self, name: &str) -> Result<Option<NetworkReach>> {
        let network = match self.docker.inspect_network(name, None).await {
            Ok(network) => network,
            Err(e) => {
                return match EngineError::from_bollard(&format!("查询网络 {name}"), e) {
                    EngineError::NotFound(_) => Ok(None),
                    other => Err(other),
                };
            }
        };
        let isolated = network
            .options
            .as_ref()
            .and_then(|o| o.get(GATEWAY_MODES[0]))
            .is_some_and(|mode| mode == "isolated");
        Ok(Some(match (network.internal.unwrap_or(false), isolated) {
            (false, _) => NetworkReach::Egress,
            (true, false) => NetworkReach::Internal,
            (true, true) => NetworkReach::Isolated,
        }))
    }

    /// 网络在宿主上的 IPv4 网关（网桥的地址）。
    pub async fn network_gateway(&self, name: &str) -> Result<String> {
        let context = format!("查询网络 {name}");
        let network = self
            .docker
            .inspect_network(name, None)
            .await
            .map_err(|e| EngineError::from_bollard(&context, e))?;
        let configs = network
            .ipam
            .and_then(|ipam| ipam.config)
            .unwrap_or_default();
        configs
            .into_iter()
            .filter_map(|config| config.gateway)
            .find(|gateway| gateway.parse::<std::net::Ipv4Addr>().is_ok())
            .ok_or_else(|| EngineError::Rejected {
                context,
                message: "网络没有 IPv4 网关".to_owned(),
            })
    }

    /// 网络的地址段（IPAM 配置里的 `Subnet`），不含网桥自己的地址（`Gateway`）。
    pub async fn network_subnets(&self, name: &str) -> Result<Vec<Subnet>> {
        let context = format!("查询网络 {name}");
        let network = self
            .docker
            .inspect_network(name, None)
            .await
            .map_err(|e| EngineError::from_bollard(&context, e))?;
        let subnets: Vec<Subnet> = network
            .ipam
            .and_then(|ipam| ipam.config)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|config| {
                let gateway = config.gateway.and_then(|g| g.parse().ok());
                Some(Subnet::parse(&config.subnet?)?.excluding(gateway))
            })
            .collect();
        if subnets.is_empty() {
            return Err(EngineError::Rejected {
                context,
                message: "网络没有地址段".to_owned(),
            });
        }
        Ok(subnets)
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

    /// 经归档接口读容器里的一个普通文件，最多 `max` 字节。路径由 Docker 在容器里解析，最后一级是软链接时
    /// 得到的是链接本身，不跟随。容器停着也能读；容器不存在时报错，文件不存在时返回 [`ContainerFile::Missing`]。
    pub async fn read_file(
        &self,
        container: &str,
        path: &str,
        max: usize,
    ) -> Result<ContainerFile> {
        if self.inspect_container(container).await?.is_none() {
            return Err(EngineError::NotFound(format!("容器 {container}")));
        }
        let context = format!("读取容器 {container} 里的 {path}");
        let options = DownloadFromContainerOptionsBuilder::default()
            .path(path)
            .build();
        let mut stream = self
            .docker
            .download_from_container(container, Some(options));
        let mut archive = Vec::new();
        while let Some(chunk) = stream.next().await {
            match chunk.map_err(|e| EngineError::from_bollard(&context, e)) {
                Ok(bytes) => archive.extend_from_slice(&bytes),
                Err(EngineError::NotFound(_)) => return Ok(ContainerFile::Missing),
                Err(other) => return Err(other),
            }
            if archive.len() > max + ARCHIVE_SLACK {
                break;
            }
        }
        single_file(&archive, max).map_err(|e| EngineError::Rejected {
            context,
            message: format!("归档无法解析：{e}"),
        })
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

    /// 列出带 `key=value` 标签、其余标签又让 `keep` 认可的容器、网络和卷（容器包括已停止的）。Docker 的标签过滤
    /// 只能要求有某个标签，不能要求没有，所以 `keep` 在这里筛。
    pub async fn labeled(
        &self,
        key: &str,
        value: &str,
        keep: impl Fn(&HashMap<String, String>) -> bool,
    ) -> Result<LabeledResources> {
        let filters = HashMap::from([("label".to_owned(), vec![format!("{key}={value}")])]);
        let kept =
            |labels: Option<&HashMap<String, String>>| keep(labels.unwrap_or(&HashMap::new()));
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
            .filter(|c| kept(c.labels.as_ref()))
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
            .filter(|n| kept(n.labels.as_ref()))
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
            .filter(|v| kept(Some(&v.labels)))
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

/// 归档接口返回的 tar 里的第一项：只接受不超过 `max` 字节的普通文件。`archive` 可能在内容中间被截断
/// （读到上限就停了），所以先看头部再读内容。
fn single_file(archive: &[u8], max: usize) -> std::io::Result<ContainerFile> {
    use std::io::Read;

    let mut archive = tar::Archive::new(archive);
    let Some(entry) = archive.entries()?.next() else {
        return Ok(ContainerFile::Missing);
    };
    let mut entry = entry?;
    if entry.header().entry_type() != tar::EntryType::Regular {
        return Ok(ContainerFile::NotRegular);
    }
    if entry.size() > max as u64 {
        return Ok(ContainerFile::TooBig);
    }
    let mut data = Vec::new();
    entry.read_to_end(&mut data)?;
    Ok(ContainerFile::Regular(data))
}

fn check_engine_version(version: &str) -> Result<()> {
    let major = version
        .split('.')
        .next()
        .and_then(|m| m.parse::<u32>().ok());
    match major {
        Some(major) if major >= MIN_ENGINE_MAJOR => Ok(()),
        Some(_) => Err(EngineError::Unsupported(format!(
            "Docker Engine {version} 太旧，核心要求 {MIN_ENGINE_MAJOR} 及以上：28 起局域网里的其他主机不能再直接访问\
             容器里没发布的端口，插件网络也才能和宿主完全隔开。请升级 Docker"
        ))),
        None => Err(EngineError::Unsupported(format!(
            "无法识别 Docker Engine 的版本 `{version}`"
        ))),
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
    use super::{ContainerFile, EngineError, check_engine_version, pull_reference, single_file};

    #[test]
    fn engines_older_than_28_are_refused() {
        for good in ["28.0.0", "28.5.1", "29.8.2", "30.0.0-rc.1"] {
            assert!(check_engine_version(good).is_ok(), "{good}");
        }
        for bad in ["27.5.1", "20.10.24", "", "dev"] {
            assert!(
                matches!(check_engine_version(bad), Err(EngineError::Unsupported(_))),
                "{bad}"
            );
        }
    }

    fn tar_of(entries: &[(&str, tar::EntryType, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, kind, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            builder.append_data(&mut header, path, *data).unwrap();
        }
        builder.into_inner().unwrap()
    }

    #[test]
    fn only_small_regular_files_are_read_from_archives() {
        use tar::EntryType::{Directory, Regular, Symlink};

        let file = tar_of(&[("key", Regular, b"k-1")]);
        assert_eq!(
            single_file(&file, 8).unwrap(),
            ContainerFile::Regular(b"k-1".to_vec())
        );
        assert_eq!(single_file(&file, 2).unwrap(), ContainerFile::TooBig);
        // 读到上限就停：内容被截断时只凭头部判断。
        assert_eq!(single_file(&file[..512], 2).unwrap(), ContainerFile::TooBig);
        for kind in [Directory, Symlink] {
            let other = tar_of(&[("key", kind, b"")]);
            assert_eq!(single_file(&other, 8).unwrap(), ContainerFile::NotRegular);
        }
    }

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
