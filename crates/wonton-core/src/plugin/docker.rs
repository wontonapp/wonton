//! Docker Engine API 的薄封装（bollard）。只暴露插件管理器用到的操作，并把错误归类：
//! Docker 不可达、没按时处理完、资源不存在、Docker 拒绝请求。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bollard::Docker;
use bollard::errors::Error as BollardError;
use bollard::models::{
    ContainerCreateBody, ContainerInspectResponse, HealthConfig, VolumeCreateRequest,
};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, DownloadFromContainerOptionsBuilder,
    KillContainerOptionsBuilder, ListContainersOptionsBuilder, ListNetworksOptionsBuilder,
    ListVolumesOptionsBuilder, RemoveContainerOptionsBuilder, RemoveVolumeOptionsBuilder,
    StopContainerOptionsBuilder, UploadToContainerOptionsBuilder,
};
use futures_util::StreamExt;
use wonton_l10n::Arg;

use super::lan::{Detector, ports};
use super::languages::Languages;
use super::network_pool::NetworkPool;
use super::requirements::Arch;
use crate::l10n;
use ipnet::Ipv4Net;

/// 核心要求的 Docker Engine 最低主版本：28 起局域网里的其他主机不能再经路由直接访问容器里没发布的端口
/// （moby#48724），内部网络也才有 `isolated` 网关模式。
pub const MIN_ENGINE_MAJOR: u32 = 28;

/// 停、删容器和删网络、卷的时限，比 bollard 默认的 120 秒长。磁盘慢时 Docker 删一个容器的层要两分钟以上，事后手动删只要
/// 2 秒（根盘满载的主机上实测，Docker 29.7.2），SD 卡这样的低配主机上也一样。
const SLOW_REQUEST: Duration = Duration::from_secs(300);

pub(super) const GATEWAY_MODES: [&str; 2] = [
    "com.docker.network.bridge.gateway_mode_ipv4",
    "com.docker.network.bridge.gateway_mode_ipv6",
];

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("{0}")]
    Unavailable(String),
    /// 连得上，但 Docker 没按时处理完。操作可能还在 Docker 里继续，结果不确定。
    #[error("{0}")]
    TimedOut(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    NotFound(String),
    /// `context` 是出错的场合，`message` 是拼好的整句（见 `from_bollard`）。
    #[error("{message}")]
    Rejected { context: String, message: String },
    /// 容器要发布的端口被宿主上别的东西占着，Docker 拒绝启动。`message` 是给主人看的那句，带上地址、端口和 Docker 的原话。
    #[error("{0}")]
    PortTaken(String),
    /// 镜像仓库连不上、镜像或标签不存在、没有权限等，`message` 是 Docker 的原话。
    #[error("{message}")]
    Pull { image: String, message: String },
}

/// 把 bollard 的错误按场合拼成给用户看的整句。
pub(super) fn from_bollard(langs: &Languages, context: &str, error: BollardError) -> EngineError {
    match error {
        BollardError::DockerResponseServerError {
            status_code: 404,
            message,
        } => {
            let what = format!("{context}：{message}");
            EngineError::NotFound(l10n::text(
                langs,
                "plugin-install-docker-not-found",
                &[("what", what.into())],
            ))
        }
        BollardError::DockerResponseServerError { message, .. } => match ports::taken(&message) {
            Some(place) => EngineError::PortTaken(match place {
                Some(place) => l10n::text(
                    langs,
                    "plugin-install-port-taken",
                    &[("place", place.into()), ("error", message.into())],
                ),
                None => l10n::text(
                    langs,
                    "plugin-install-port-taken-unknown",
                    &[("error", message.into())],
                ),
            }),
            None => rejected(langs, context, message),
        },
        BollardError::DockerStreamError { error: message } => rejected(langs, context, message),
        BollardError::RequestTimeoutError => EngineError::TimedOut(l10n::text(
            langs,
            "plugin-install-docker-timed-out",
            &[("context", context.into())],
        )),
        other => EngineError::Unavailable(l10n::text(
            langs,
            "plugin-install-docker-unavailable",
            &[("error", format!("{context}：{other}").into())],
        )),
    }
}

/// Docker 拒绝了请求，原话带上。
fn rejected(langs: &Languages, context: &str, message: String) -> EngineError {
    let error = l10n::text(
        langs,
        "plugin-install-docker-rejected",
        &[("context", context.into()), ("error", message.into())],
    );
    EngineError::Rejected {
        context: context.to_owned(),
        message: error,
    }
}

/// Docker 守护进程的版本和宿主架构。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    pub version: String,
    pub arch: String,
}

/// Docker 报告的主机情况。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInfo {
    /// Docker `/info` 的 `Name`：daemon 所在主机的名字（安装时登记的就是宿主 hostname）。
    pub name: Option<String>,
    pub os_type: String,
    pub arch: String,
    pub runtimes: Vec<String>,
    pub memory_limit: Option<bool>,
    pub swap_limit: Option<bool>,
    pub cpu_shares: Option<bool>,
    pub pids_limit: Option<bool>,
    pub cgroup_driver: Option<String>,
    /// Docker `/info` 报告的逻辑 CPU 数；主人设置 CPU 上限时必须使用它。
    pub ncpu: Option<i64>,
    pub cgroup_version: Option<String>,
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

/// 镜像的平台列表。`size` 只在单个清单（不是多架构列表）时有，是压缩后的大小。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlatforms {
    pub architectures: Vec<String>,
    pub size: Option<u64>,
}

impl ImagePlatforms {
    /// Docker 报告的架构（`x86_64`、`aarch64`）和镜像仓库里的（`amd64`、`arm64`）写法不同，两边都归一。
    pub fn supports(&self, arch: &str) -> bool {
        let arch = Arch::from_docker(arch);
        arch.is_some_and(|arch| {
            self.architectures
                .iter()
                .any(|candidate| Arch::from_docker(candidate) == Some(arch))
        })
    }

    /// 没有平台列表时不给大小：多架构的列表大小不是任何一个镜像的。
    pub fn compressed_size(&self) -> Option<u64> {
        self.size
    }
}

const MEDIA_TYPE_INDEX: &str = "application/vnd.oci.image.index.v1+json";
const MEDIA_TYPE_LIST: &str = "application/vnd.docker.distribution.manifest.list.v2+json";

#[derive(Clone)]
pub struct Engine {
    pub(crate) docker: Docker,
    /// 一次性容器一个跑完再跑下一个：局域网检测和出口容器的探测都可能同时要跑，容器名是固定的。
    pub(super) one_shot: Arc<tokio::sync::Mutex<()>>,
    /// 选网段加建网络全程串行：本进程的并发建网络各自现查会挑到同一段，Docker 对
    /// 重叠的并发建网全部拒绝，重试也是一起撞（20 个并发网络实测 10 次重试都撞不出一个）。
    /// 锁只管本进程；外部程序同时建的仍靠重试。
    pub(super) reserved_creation: Arc<tokio::sync::Mutex<()>>,
    /// 渲染前预留的网段（见 [`Self::reserve_subnets`]）：别的建网络跳过它们，渲染失败放掉。
    pub(super) held_subnets: Arc<std::sync::Mutex<Vec<Ipv4Net>>>,
    pub(super) network_pool: NetworkPool,
    pub(super) network_detector: Detector,
    /// 语言文件和主人的用户名，按它找主人的语言（[`Self::set_languages`]）；没接上时按默认语言写。
    languages: Option<(std::path::PathBuf, String)>,
    /// 测试用的建容器记录（`for_test` 才有）：观察签发路径有没有真的试图起容器。
    pub(crate) creations: Option<Arc<std::sync::Mutex<Vec<String>>>>,
}

type Result<T> = std::result::Result<T, EngineError>;

mod networks;

impl Engine {
    /// 按 `DOCKER_HOST` 连接，未设置时使用本机 socket。
    pub fn connect() -> Result<Self> {
        let docker = Docker::connect_with_defaults().map_err(|e| {
            EngineError::Unavailable(l10n::text(
                &Languages::default(),
                "plugin-install-docker-unavailable",
                &[("error", e.to_string().into())],
            ))
        })?;
        Ok(Self::new(docker))
    }

    /// 不连守护进程的客户端，给从来不碰 Docker 的测试用：bollard 只检查路径存在，
    /// 建一个空的 socket 文件就够，没有 Docker 的机器上也建得出来。
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        let path = std::env::temp_dir().join(format!("wonton-test-{}.sock", std::process::id()));
        std::fs::write(&path, b"").expect("写测试 socket 占位文件");
        let mut engine = Self::new(
            Docker::connect_with_unix(
                &format!("unix://{}", path.display()),
                120,
                bollard::API_DEFAULT_VERSION,
            )
            .expect("构造 Docker 客户端"),
        );
        engine.creations = Some(Arc::default());
        engine
    }

    /// `for_test` 引擎里试过的建容器命令（只记测试路径；生产引擎没有记录）。
    #[cfg(test)]
    pub(crate) fn recorded_creations(&self) -> Vec<String> {
        self.creations
            .as_ref()
            .map(|creations| creations.lock().unwrap().clone())
            .unwrap_or_default()
    }

    fn new(docker: Docker) -> Self {
        Self {
            docker,
            one_shot: Arc::default(),
            reserved_creation: Arc::default(),
            held_subnets: Arc::default(),
            network_pool: NetworkPool::default(),
            network_detector: Detector::Here,
            languages: None,
            creations: None,
        }
    }

    /// 语言文件（[`super::store::Store::people_languages`]）和主人的用户名；读不到时按默认语言写
    /// （接线由核心启动时做）。
    pub fn set_languages(&mut self, file: std::path::PathBuf, owner: String) {
        self.languages = Some((file, owner));
    }

    pub(super) fn owner_languages(&self) -> Languages {
        match &self.languages {
            Some((file, owner)) => Languages::from_header(
                super::store::read_people_languages(file)
                    .get(owner)
                    .map(String::as_str),
            ),
            None => Languages::default(),
        }
    }

    fn fail(&self, id: &str, args: &[(&str, Arg<'_>)], error: BollardError) -> EngineError {
        let langs = self.owner_languages();
        from_bollard(&langs, &l10n::text(&langs, id, args), error)
    }

    fn fail_name(&self, id: &str, name: &str, error: BollardError) -> EngineError {
        self.fail(id, &[("name", name.into())], error)
    }

    fn fail_network(
        &self,
        id: &str,
        container: &str,
        network: &str,
        error: BollardError,
    ) -> EngineError {
        self.fail(
            id,
            &[("container", container.into()), ("network", network.into())],
            error,
        )
    }

    fn net_context(&self, name: &str) -> (Languages, String) {
        let langs = self.owner_languages();
        let context = l10n::text(
            &langs,
            "plugin-install-docker-context-network",
            &[("name", name.into())],
        );
        (langs, context)
    }

    /// Docker 可能处理很久的请求用这个客户端，见 [`SLOW_REQUEST`]。
    fn patient(&self) -> Docker {
        self.docker.clone().with_timeout(SLOW_REQUEST)
    }

    /// 测试用：换掉普通请求的时限，不用真等两分钟。
    #[cfg(test)]
    pub(crate) fn with_request_limit_for_test(self, limit: Duration) -> Self {
        Self {
            docker: self.docker.with_timeout(limit),
            ..self
        }
    }

    /// 测试用：连到给定地址上的 Engine API（测试里的假 Docker）。
    #[cfg(test)]
    pub(crate) fn http_for_test(url: &str) -> Self {
        let docker = Docker::connect_with_http(url, 5, bollard::API_DEFAULT_VERSION)
            .expect("构造测试用 Docker 客户端");
        Self::new(docker)
    }

    /// 测试用：指向一个不存在的 Docker，用于不需要 Docker 的网关测试。
    #[cfg(test)]
    pub(crate) fn unreachable_for_test() -> Self {
        let docker =
            Docker::connect_with_http("http://127.0.0.1:9", 1, bollard::API_DEFAULT_VERSION)
                .expect("构造测试用 Docker 客户端");
        Self::new(docker)
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
            .map_err(|e| self.fail("plugin-install-docker-context-version", &[], e))?;
        Ok(ServerInfo {
            version: version.version.unwrap_or_default(),
            arch: version.arch.unwrap_or_default(),
        })
    }

    /// Docker 登记的 OCI 运行时的名字（`runc`，装了 NVIDIA Container Toolkit 时还有 `nvidia`）。
    pub async fn runtimes(&self) -> Result<Vec<String>> {
        Ok(self.host_info().await?.runtimes)
    }

    pub async fn host_info(&self) -> Result<HostInfo> {
        let info = self
            .docker
            .info()
            .await
            .map_err(|e| self.fail("plugin-install-docker-context-host-info", &[], e))?;
        let mut runtimes: Vec<String> = info.runtimes.unwrap_or_default().into_keys().collect();
        runtimes.sort();
        Ok(HostInfo {
            name: info.name,
            os_type: info.os_type.unwrap_or_default(),
            arch: info.architecture.unwrap_or_default(),
            runtimes,
            memory_limit: info.memory_limit,
            swap_limit: info.swap_limit,
            cpu_shares: info.cpu_shares,
            pids_limit: info.pids_limit,
            ncpu: info.ncpu,
            cgroup_driver: info.cgroup_driver.map(|value| value.to_string()),
            cgroup_version: info.cgroup_version.map(|value| value.to_string()),
        })
    }
    /// 核心启动时检查 Docker：连得上，而且 Engine 版本不低于 [`MIN_ENGINE_MAJOR`]。低于时报错，不降级运行：
    /// 插件网络的隔离靠的是新版的行为。
    pub async fn check_server(&self) -> Result<ServerInfo> {
        let info = self.server().await?;
        check_engine_version(&self.owner_languages(), &info.version)?;
        Ok(info)
    }

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

    pub async fn create_container(&self, name: &str, body: ContainerCreateBody) -> Result<()> {
        if let Some(creations) = &self.creations {
            let command = body.cmd.clone().unwrap_or_default().join(" ");
            creations.lock().unwrap().push(command);
        }
        let options = CreateContainerOptionsBuilder::default().name(name).build();
        let response = self
            .docker
            .create_container(Some(options), body)
            .await
            .map_err(|e| {
                self.fail_name("plugin-install-docker-context-create-container", name, e)
            })?;
        for warning in response.warnings {
            tracing::warn!(container = name, warning = %warning, "Docker 创建容器时给出警告");
        }
        Ok(())
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
            .map_err(|e| {
                self.fail(
                    "plugin-install-docker-context-upload-archive",
                    &[("container", container.into())],
                    e,
                )
            })
    }

    /// 经归档接口读容器里的一个普通文件，最多 `max` 字节。路径由 Docker 在容器里解析，最后一级是软链接时
    /// 得到的是链接本身，不跟随。容器停着也能读；容器不存在时报错，文件不存在时返回 [`ContainerFile::Missing`]。
    pub async fn read_file(
        &self,
        container: &str,
        path: &str,
        max: usize,
    ) -> Result<ContainerFile> {
        let langs = self.owner_languages();
        if self.inspect_container(container).await?.is_none() {
            let missing = l10n::text(
                &langs,
                "plugin-install-docker-context-container-missing",
                &[("container", container.into())],
            );
            return Err(EngineError::NotFound(missing));
        }
        let context = l10n::text(
            &langs,
            "plugin-install-docker-context-read-file",
            &[("container", container.into()), ("path", path.into())],
        );
        let options = DownloadFromContainerOptionsBuilder::default()
            .path(path)
            .build();
        let mut stream = self
            .docker
            .download_from_container(container, Some(options));
        let mut archive = Vec::new();
        while let Some(chunk) = stream.next().await {
            match chunk.map_err(|e| from_bollard(&langs, &context, e)) {
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
            message: l10n::text(
                &langs,
                "plugin-install-docker-archive-unparseable",
                &[("error", e.to_string().into())],
            ),
        })
    }

    pub async fn start_container(&self, name: &str) -> Result<()> {
        self.docker
            .start_container(name, None)
            .await
            .map_err(|e| self.fail_name("plugin-install-docker-context-start-container", name, e))
    }
    /// 给容器发信号（如 `SIGUSR1`）；返回错误时信号可能已经发出，调用方不要盲目重试。
    pub async fn kill_container(&self, name: &str, signal: &str) -> Result<()> {
        let options = KillContainerOptionsBuilder::default()
            .signal(signal)
            .build();
        self.docker
            .kill_container(name, Some(options))
            .await
            .map_err(|e| {
                self.fail(
                    "plugin-install-docker-context-kill-container",
                    &[("name", name.into()), ("signal", signal.into())],
                    e,
                )
            })
    }

    /// 查询容器；不存在时返回 `None`。
    pub async fn inspect_container(&self, name: &str) -> Result<Option<ContainerInspectResponse>> {
        match self.docker.inspect_container(name, None).await {
            Ok(info) => Ok(Some(info)),
            Err(e) => {
                match self.fail_name("plugin-install-docker-context-inspect-container", name, e) {
                    EngineError::NotFound(_) => Ok(None),
                    other => Err(other),
                }
            }
        }
    }

    /// 停止但保留容器及卷；已停止视为成功。
    pub async fn stop_container(&self, name: &str) -> Result<()> {
        let stop = StopContainerOptionsBuilder::default().t(10).build();
        match self.patient().stop_container(name, Some(stop)).await {
            Ok(()) => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 304, ..
            }) => Ok(()),
            Err(error) => {
                Err(self.fail_name("plugin-install-docker-context-stop-container", name, error))
            }
        }
    }

    /// 停止并删除容器（连同匿名卷）；不存在视为成功。
    pub async fn remove_container(&self, name: &str) -> Result<()> {
        let stop = StopContainerOptionsBuilder::default().t(10).build();
        let docker = self.patient();
        let stopped = docker
            .stop_container(name, Some(stop))
            .await
            .map_err(|e| self.fail_name("plugin-install-docker-context-stop-container", name, e));
        match stopped {
            Ok(()) | Err(EngineError::NotFound(_)) | Err(EngineError::Rejected { .. }) => {}
            Err(other) => return Err(other),
        }
        let options = RemoveContainerOptionsBuilder::default()
            .force(true)
            .v(true)
            .build();
        ignore_missing(
            docker
                .remove_container(name, Some(options))
                .await
                .map_err(|e| {
                    self.fail_name("plugin-install-docker-context-remove-container", name, e)
                }),
        )
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
            .map_err(|e| self.fail("plugin-install-docker-context-list-containers", &[], e))?
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
            .map_err(|e| self.fail("plugin-install-docker-context-list-networks", &[], e))?
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
            .map_err(|e| self.fail("plugin-install-docker-context-list-volumes", &[], e))?
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

fn check_engine_version(langs: &Languages, version: &str) -> Result<()> {
    let major = version
        .split('.')
        .next()
        .and_then(|m| m.parse::<u32>().ok());
    match major {
        Some(major) if major >= MIN_ENGINE_MAJOR => Ok(()),
        Some(_) => Err(EngineError::Unsupported(l10n::text(
            langs,
            "plugin-install-docker-engine-too-old",
            &[
                ("version", version.into()),
                ("min", MIN_ENGINE_MAJOR.into()),
            ],
        ))),
        None => Err(EngineError::Unsupported(l10n::text(
            langs,
            "plugin-install-docker-engine-version-unrecognized",
            &[("version", version.into())],
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
    use super::{ContainerFile, EngineError, check_engine_version, single_file};
    use crate::plugin::languages::Languages;
    use crate::plugin::registry_mirror::reference::pull_reference;

    #[test]
    fn engines_older_than_28_are_refused() {
        for good in ["28.0.0", "28.5.1", "29.8.2", "30.0.0-rc.1"] {
            assert!(
                check_engine_version(&Languages::default(), good).is_ok(),
                "{good}"
            );
        }
        for bad in ["27.5.1", "20.10.24", "", "dev"] {
            assert!(
                matches!(
                    check_engine_version(&Languages::default(), bad),
                    Err(EngineError::Unsupported(_))
                ),
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
