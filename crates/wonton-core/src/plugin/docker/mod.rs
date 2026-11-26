//! Docker Engine API 的薄封装（bollard）。只暴露插件管理器用到的操作，并把错误归类：
//! Docker 不可达、没按时处理完、资源不存在、Docker 拒绝请求。

use std::sync::Arc;
use std::time::Duration;

use bollard::Docker;
use bollard::errors::Error as BollardError;
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

/// 创建、停、删容器和删网络、卷的时限，比 bollard 默认的 120 秒长。磁盘慢时 Docker 删一个容器的层要两分钟以上，
/// 事后手动删只要 2 秒（根盘满载的主机上实测，Docker 29.7.2），SD 卡这样的低配主机上也一样；创建容器同样受满盘拖慢
/// （11-14 的安装 504）。宿主满载时建容器超过 120 秒，默认时限就会把还活着的操作误报成超时。
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
    /// 慢请求（停、删、创建容器）的时限，见 [`SLOW_REQUEST`]。测试改短用。
    slow_request: Duration,
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

mod containers;
mod files;
mod images;
mod info;
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
            slow_request: SLOW_REQUEST,
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
        self.docker.clone().with_timeout(self.slow_request)
    }

    /// 测试用：换掉普通请求的时限，不用真等两分钟。
    #[cfg(test)]
    pub(crate) fn with_request_limit_for_test(self, limit: Duration) -> Self {
        Self {
            docker: self.docker.with_timeout(limit),
            ..self
        }
    }

    /// 测试用：换掉慢请求的时限，让超时用例不用等五分钟。
    #[cfg(test)]
    pub(crate) fn with_slow_request_limit_for_test(self, limit: Duration) -> Self {
        Self {
            slow_request: limit,
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
}

fn ignore_missing(result: Result<()>) -> Result<()> {
    match result {
        Err(EngineError::NotFound(_)) => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::files::single_file;
    use super::info::check_engine_version;
    use super::{ContainerFile, EngineError};
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
