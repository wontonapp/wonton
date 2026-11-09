//! 升级要主机做的事：Docker、compose、磁盘和时钟。真的实现在 [`super::docker_host`]，行为测试用假的。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::future::BoxFuture;

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Failed(String),
}

pub type Result<T> = std::result::Result<T, HostError>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageInfo {
    pub id: String,
    pub repo_digests: Vec<String>,
    pub labels: HashMap<String, String>,
    pub size: u64,
}

/// 下载进展：已经下载的字节数和总字节数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

/// 一次性容器挂载的东西。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mount {
    Bind {
        source: PathBuf,
        target: PathBuf,
        read_only: bool,
    },
    Volume {
        name: String,
        target: PathBuf,
        read_only: bool,
    },
}

impl Mount {
    /// 容器内外路径相同的绑定挂载。
    pub fn same_path(path: &Path, read_only: bool) -> Self {
        Self::Bind {
            source: path.to_owned(),
            target: path.to_owned(),
            read_only,
        }
    }
}

/// 在某个镜像里跑一次的容器。名字固定：升级器重启后先清掉上次的。不联网。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OneShot {
    pub name: String,
    pub image: String,
    pub entrypoint: Option<Vec<String>>,
    pub cmd: Vec<String>,
    pub mounts: Vec<Mount>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OneShotOutput {
    pub exit_code: i64,
    pub stdout: String,
    pub stderr: String,
}

/// compose 项目里的一个容器。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceContainer {
    pub id: String,
    pub name: String,
    pub service: String,
    /// `Config.Image`：compose 写的引用。
    pub image: String,
    /// 容器实际跑着的镜像 ID（inspect 的 `Image`）：引用会被移走，它不会。
    pub image_id: String,
    /// `running`、`exited`、`restarting` 等。
    pub status: String,
    /// 没有健康检查时是 `None`。
    pub health: Option<String>,
    pub exit_code: i64,
    pub restart_count: i64,
    /// 设了重启策略的是长期服务，没设的是一次性服务（跨版本的约定）。
    pub long_running: bool,
    /// 用到的命名卷。
    pub volumes: Vec<String>,
}

/// 核心管理的其他组件（ADR-0025）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Component {
    /// compose 项目，名字就是组件名。
    Project(String),
    /// 经 Docker API 建的容器。
    Container {
        component: String,
        container: String,
    },
}

/// 查一个容器。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerInfo {
    pub image_id: String,
    /// `Config.Image`。
    pub config_image: String,
    /// 在运行，或者正在被 Docker 重启。
    pub running: bool,
    /// 重启策略是 `no`：退出后不会再被拉起来。
    pub restart_disabled: bool,
}

/// 建升级器容器要的东西。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdaterSpec {
    /// 正在运行的核心的镜像 ID。
    pub image_id: String,
    pub cmd: Vec<String>,
    pub mounts: Vec<Mount>,
}

pub trait Host: Send + Sync {
    /// 现在的 Unix 时间（秒）。状态文件里的时限都按它算。
    fn now(&self) -> u64;
    fn sleep(&self, duration: Duration) -> BoxFuture<'_, ()>;
    /// 这个目录所在的磁盘还能用多少字节。
    fn free_bytes(&self, path: &Path) -> Result<u64>;

    /// 本机的镜像；没有时是 `None`。
    fn image<'a>(&'a self, reference: &'a str) -> BoxFuture<'a, Result<Option<ImageInfo>>>;
    /// 经 Docker 的分发接口取 tag 指向的 digest。
    fn registry_digest<'a>(&'a self, source: &'a str) -> BoxFuture<'a, Result<String>>;
    /// 拉镜像，连续 [`PULL_STALL`](super::state::PULL_STALL) 没有进展算失败。
    fn pull<'a>(
        &'a self,
        image: &'a str,
        progress: &'a (dyn Fn(Progress) + Send + Sync),
    ) -> BoxFuture<'a, Result<()>>;
    fn remove_image<'a>(&'a self, image: &'a str) -> BoxFuture<'a, Result<()>>;

    fn run_once<'a>(&'a self, job: &'a OneShot) -> BoxFuture<'a, Result<OneShotOutput>>;
    /// 这个 compose 项目里的容器，包括已停止的。
    fn project_containers<'a>(
        &'a self,
        project: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ServiceContainer>>>;
    fn components(&self) -> BoxFuture<'_, Result<Vec<Component>>>;
    fn stop<'a>(&'a self, container: &'a str) -> BoxFuture<'a, Result<()>>;
    fn remove_container<'a>(&'a self, container: &'a str) -> BoxFuture<'a, Result<()>>;
    /// `docker-compose -f <安装目录>/compose.yaml up -d --wait --wait-timeout <wait> --remove-orphans`。
    fn compose_up<'a>(&'a self, install_dir: &'a Path, wait: Duration)
    -> BoxFuture<'a, Result<()>>;
    /// `docker-compose -p <项目> down`，不加 `-v`。
    fn compose_down<'a>(&'a self, project: &'a str) -> BoxFuture<'a, Result<()>>;

    /// 查容器；不存在时是 `None`。
    fn container<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Option<ContainerInfo>>>;
    fn disable_restart<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<()>>;
    /// 建好并启动升级器容器，重启策略 `unless-stopped`，不联网。
    fn start_updater<'a>(&'a self, spec: &'a UpdaterSpec) -> BoxFuture<'a, Result<()>>;
}
