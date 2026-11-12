//! 真的主机：经 Docker API 和镜像里的 `docker-compose` 文件操作（ADR-0025）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bollard::container::LogOutput;
use bollard::errors::Error as BollardError;
use bollard::models::{
    ContainerCreateBody, ContainerInspectResponse, ContainerUpdateBody, HostConfig,
    Mount as DockerMount, MountBindOptions, MountType, RestartPolicy, RestartPolicyNameEnum,
};
use bollard::query_parameters::{
    ListContainersOptionsBuilder, LogsOptionsBuilder, RemoveImageOptionsBuilder,
    StopContainerOptionsBuilder,
};
use futures_util::StreamExt;
use futures_util::future::BoxFuture;

use super::host::{
    Component, ContainerInfo, Host, HostError, ImageInfo, Mount, OneShot, OneShotOutput, Progress,
    Result, ServiceContainer, UpdaterSpec,
};
use super::state::PULL_STALL;
use super::{LABEL_COMPONENT, LABEL_MANAGED_BY, PROJECT, UPDATER_CONTAINER};
use crate::plugin::docker::Engine;

/// 核心镜像里的 compose（`deploy/dev/core/Dockerfile`）。
pub const COMPOSE_BIN: &str = "/usr/local/bin/docker-compose";
/// `--wait-timeout` 之外还要等停止宽限（Tuwunel 5 分钟）：超过这么久还没退就杀掉 compose。
const COMPOSE_GRACE: Duration = Duration::from_secs(330);
/// compose 的 `down` 的时限。
const COMPOSE_DOWN: Duration = Duration::from_secs(330);
/// 错误信息里只留输出的最后这么多字节。
const OUTPUT_TAIL: usize = 4 << 10;
const STOP_SECS: i32 = 30;

pub struct DockerHost {
    engine: Engine,
    compose: PathBuf,
}

impl DockerHost {
    pub fn new(engine: Engine) -> Self {
        Self::with_compose(engine, PathBuf::from(COMPOSE_BIN))
    }

    pub fn with_compose(engine: Engine, compose: PathBuf) -> Self {
        Self { engine, compose }
    }

    /// compose 里环境变量优先于 `.env`：不能带任何 `WONTON_*`，带着旧的 `WONTON_CORE_IMAGE` 就换不了镜像。
    async fn compose(&self, args: &[String], limit: Duration) -> Result<()> {
        let mut command = tokio::process::Command::new(&self.compose);
        command
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for key in ["DOCKER_HOST", "HOME"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        let child = command
            .spawn()
            .map_err(|e| failed(format!("运行 {} 失败：{e}", self.compose.display())))?;
        let output = tokio::time::timeout(limit, child.wait_with_output())
            .await
            .map_err(|_| failed(format!("compose 超过 {} 秒还没结束", limit.as_secs())))?
            .map_err(|e| failed(format!("等 compose 结束失败：{e}")))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(failed(format!(
            "compose 失败（{}）：{}",
            output.status,
            tail(stderr.trim())
        )))
    }

    async fn inspect(&self, name: &str) -> Result<Option<ContainerInspectResponse>> {
        self.engine
            .inspect_container(name)
            .await
            .map_err(engine_error)
    }

    async fn output_of(&self, name: &str) -> (String, String) {
        let options = LogsOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .build();
        let mut stream = self.engine.docker.logs(name, Some(options));
        let (mut stdout, mut stderr) = (String::new(), String::new());
        while let Some(Ok(chunk)) = stream.next().await {
            match chunk {
                LogOutput::StdOut { message } => {
                    stdout.push_str(&String::from_utf8_lossy(&message))
                }
                LogOutput::StdErr { message } => {
                    stderr.push_str(&String::from_utf8_lossy(&message))
                }
                _ => {}
            }
        }
        (stdout, tail(stderr.trim()).to_owned())
    }

    async fn run_job(&self, job: &OneShot) -> Result<OneShotOutput> {
        self.engine
            .remove_container(&job.name)
            .await
            .map_err(engine_error)?;
        let body = ContainerCreateBody {
            image: Some(job.image.clone()),
            entrypoint: job.entrypoint.clone(),
            cmd: Some(job.cmd.clone()),
            host_config: Some(HostConfig {
                network_mode: Some("none".to_owned()),
                mounts: Some(job.mounts.iter().map(docker_mount).collect()),
                ..Default::default()
            }),
            ..Default::default()
        };
        self.engine
            .create_container(&job.name, body)
            .await
            .map_err(engine_error)?;
        self.engine
            .start_container(&job.name)
            .await
            .map_err(engine_error)?;
        // 退出码不等于 0 时 wait 接口报错，这里只等它结束，退出码另外查。
        let mut wait = self.engine.docker.wait_container(&job.name, None);
        let waited = tokio::time::timeout(job.timeout, wait.next()).await;
        drop(wait);
        if waited.is_err() {
            let message = format!(
                "容器 {} 超过 {} 秒还没结束",
                job.name,
                job.timeout.as_secs()
            );
            self.engine
                .remove_container(&job.name)
                .await
                .map_err(engine_error)?;
            return Err(failed(message));
        }
        let exit_code = self
            .inspect(&job.name)
            .await?
            .and_then(|c| c.state)
            .and_then(|s| s.exit_code)
            .ok_or_else(|| failed(format!("查不到容器 {} 的退出码", job.name)))?;
        let (stdout, stderr) = self.output_of(&job.name).await;
        self.engine
            .remove_container(&job.name)
            .await
            .map_err(engine_error)?;
        Ok(OneShotOutput {
            exit_code,
            stdout,
            stderr,
        })
    }

    async fn pull_image(
        &self,
        image: &str,
        progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<()> {
        let options = bollard::query_parameters::CreateImageOptionsBuilder::default()
            .from_image(image)
            .build();
        let mut stream = self.engine.docker.create_image(Some(options), None, None);
        let mut layers: HashMap<String, (u64, u64)> = HashMap::new();
        loop {
            let item = tokio::time::timeout(PULL_STALL, stream.next())
                .await
                .map_err(|_| failed(format!("拉 {image} 连续 5 分钟没有进展")))?;
            let Some(item) = item else {
                return Ok(());
            };
            let item = item.map_err(|e| failed(bollard_message(e)))?;
            if let (Some(id), Some(detail)) = (item.id, item.progress_detail) {
                let (current, total) = (detail.current.unwrap_or(0), detail.total.unwrap_or(0));
                if total > 0 {
                    layers.insert(id, (current.max(0) as u64, total as u64));
                    progress(Progress {
                        done: layers.values().map(|(c, _)| c).sum(),
                        total: layers.values().map(|(_, t)| t).sum(),
                    });
                }
            }
        }
    }

    async fn list_project(&self, project: &str) -> Result<Vec<ServiceContainer>> {
        let filters = HashMap::from([(
            "label".to_owned(),
            vec![format!("com.docker.compose.project={project}")],
        )]);
        let options = ListContainersOptionsBuilder::default()
            .all(true)
            .filters(&filters)
            .build();
        let listed = self
            .engine
            .docker
            .list_containers(Some(options))
            .await
            .map_err(|e| failed(bollard_message(e)))?;
        let mut containers = Vec::new();
        for id in listed.into_iter().filter_map(|c| c.id) {
            // 列表和查询之间容器可能刚好被换掉了。
            if let Some(info) = self.inspect(&id).await? {
                containers.push(service_container(id, info));
            }
        }
        Ok(containers)
    }

    async fn list_components(&self) -> Result<Vec<Component>> {
        let filters =
            HashMap::from([("label".to_owned(), vec![format!("{LABEL_MANAGED_BY}=core")])]);
        let options = ListContainersOptionsBuilder::default()
            .all(true)
            .filters(&filters)
            .build();
        let listed = self
            .engine
            .docker
            .list_containers(Some(options))
            .await
            .map_err(|e| failed(bollard_message(e)))?;
        let mut components = Vec::new();
        for container in listed {
            let labels = container.labels.unwrap_or_default();
            let found = if let Some(project) = labels.get("com.docker.compose.project") {
                (project != PROJECT).then(|| Component::Project(project.clone()))
            } else if let (Some(component), Some(id)) = (labels.get(LABEL_COMPONENT), container.id)
            {
                Some(Component::Container {
                    component: component.clone(),
                    container: id,
                })
            } else {
                None
            };
            if let Some(found) = found
                && !components.contains(&found)
            {
                components.push(found);
            }
        }
        Ok(components)
    }

    async fn stop_container(&self, name: &str) -> Result<()> {
        let options = StopContainerOptionsBuilder::default().t(STOP_SECS).build();
        match self.engine.docker.stop_container(name, Some(options)).await {
            Ok(()) => Ok(()),
            // 已经停了（304）或者不存在（404）。
            Err(BollardError::DockerResponseServerError {
                status_code: 304 | 404,
                ..
            }) => Ok(()),
            Err(e) => Err(failed(bollard_message(e))),
        }
    }

    async fn create_updater(&self, spec: &UpdaterSpec) -> Result<()> {
        let body = ContainerCreateBody {
            image: Some(spec.image_id.clone()),
            cmd: Some(spec.cmd.clone()),
            host_config: Some(HostConfig {
                network_mode: Some("none".to_owned()),
                mounts: Some(spec.mounts.iter().map(docker_mount).collect()),
                restart_policy: Some(RestartPolicy {
                    name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                    maximum_retry_count: None,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        self.engine
            .create_container(UPDATER_CONTAINER, body)
            .await
            .map_err(engine_error)?;
        self.engine
            .start_container(UPDATER_CONTAINER)
            .await
            .map_err(engine_error)
    }
}

fn failed(message: String) -> HostError {
    HostError::Failed(message)
}

fn engine_error(e: crate::plugin::docker::EngineError) -> HostError {
    match e {
        crate::plugin::docker::EngineError::NotFound(m) => HostError::NotFound(m),
        other => HostError::Failed(other.to_string()),
    }
}

fn bollard_message(e: BollardError) -> String {
    match e {
        BollardError::DockerResponseServerError { message, .. }
        | BollardError::DockerStreamError { error: message } => message,
        other => other.to_string(),
    }
}

/// 字符串末尾最多 [`OUTPUT_TAIL`] 字节，不切开字符。
fn tail(text: &str) -> &str {
    let mut start = text.len().saturating_sub(OUTPUT_TAIL);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

fn docker_mount(mount: &Mount) -> DockerMount {
    match mount {
        Mount::Bind {
            source,
            target,
            read_only,
        } => DockerMount {
            typ: Some(MountType::BIND),
            source: Some(source.display().to_string()),
            target: Some(target.display().to_string()),
            read_only: Some(*read_only),
            bind_options: Some(MountBindOptions {
                create_mountpoint: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        },
        Mount::Volume {
            name,
            target,
            read_only,
        } => DockerMount {
            typ: Some(MountType::VOLUME),
            source: Some(name.clone()),
            target: Some(target.display().to_string()),
            read_only: Some(*read_only),
            ..Default::default()
        },
    }
}

fn service_container(id: String, info: ContainerInspectResponse) -> ServiceContainer {
    let config = info.config.unwrap_or_default();
    let labels = config.labels.unwrap_or_default();
    let state = info.state.unwrap_or_default();
    let policy = info
        .host_config
        .and_then(|h| h.restart_policy)
        .and_then(|p| p.name);
    let long_running = !matches!(
        policy,
        None | Some(RestartPolicyNameEnum::EMPTY | RestartPolicyNameEnum::NO)
    );
    ServiceContainer {
        id,
        name: info
            .name
            .unwrap_or_default()
            .trim_start_matches('/')
            .to_owned(),
        service: labels
            .get("com.docker.compose.service")
            .cloned()
            .unwrap_or_default(),
        image: config.image.unwrap_or_default(),
        status: state.status.map(|s| s.to_string()).unwrap_or_default(),
        health: state.health.and_then(|h| h.status).map(|s| s.to_string()),
        exit_code: state.exit_code.unwrap_or(0),
        restart_count: info.restart_count.unwrap_or(0),
        long_running,
        volumes: info
            .mounts
            .unwrap_or_default()
            .into_iter()
            .filter(|m| {
                m.typ
                    .as_deref()
                    .is_some_and(|t| t.eq_ignore_ascii_case("volume"))
            })
            .filter_map(|m| m.name)
            .collect(),
    }
}

impl Host for DockerHost {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }

    fn sleep(&self, duration: Duration) -> BoxFuture<'_, ()> {
        Box::pin(tokio::time::sleep(duration))
    }

    fn free_bytes(&self, path: &Path) -> Result<u64> {
        let stats = rustix::fs::statvfs(path)
            .map_err(|e| failed(format!("查 {} 的磁盘空间失败：{e}", path.display())))?;
        Ok(stats.f_bavail.saturating_mul(stats.f_frsize.max(1)))
    }

    fn image<'a>(&'a self, reference: &'a str) -> BoxFuture<'a, Result<Option<ImageInfo>>> {
        Box::pin(async move {
            match self.engine.docker.inspect_image(reference).await {
                Ok(info) => Ok(Some(ImageInfo {
                    id: info.id.unwrap_or_default(),
                    repo_digests: info.repo_digests.unwrap_or_default(),
                    labels: info.config.and_then(|c| c.labels).unwrap_or_default(),
                    size: info.size.unwrap_or(0).max(0) as u64,
                })),
                Err(BollardError::DockerResponseServerError {
                    status_code: 404, ..
                }) => Ok(None),
                // containerd 存储里，索引对同一架构列了几个变体（Tuwunel 的 amd64、amd64/v2、amd64/v3），本机只有别的变体时，
                // 这个接口回 200，但 Config 和 RootFS 是空的，bollard 解析失败（实测 Docker 29.7.2、bollard 0.21.1）。
                // 容器照样起得来，不用为此多拉一次。这样的只有系统服务的镜像，它们只查在不在。
                Err(BollardError::JsonDataError { .. }) => Ok(Some(ImageInfo::default())),
                Err(e) => Err(failed(bollard_message(e))),
            }
        })
    }

    fn registry_digest<'a>(&'a self, source: &'a str) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let inspected = self
                .engine
                .docker
                .inspect_registry_image(source, None)
                .await
                .map_err(|e| failed(bollard_message(e)))?;
            inspected
                .descriptor
                .digest
                .ok_or_else(|| failed(format!("分发接口没有给 {source} 的 digest")))
        })
    }

    fn pull<'a>(
        &'a self,
        image: &'a str,
        progress: &'a (dyn Fn(Progress) + Send + Sync),
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.pull_image(image, progress))
    }

    fn remove_image<'a>(&'a self, image: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // 不强删：还有容器用着它时 Docker 拒绝，这正是想要的。
            let options = RemoveImageOptionsBuilder::default().build();
            match self
                .engine
                .docker
                .remove_image(image, Some(options), None)
                .await
            {
                Ok(_)
                | Err(BollardError::DockerResponseServerError {
                    status_code: 404, ..
                }) => Ok(()),
                Err(e) => Err(failed(bollard_message(e))),
            }
        })
    }

    fn run_once<'a>(&'a self, job: &'a OneShot) -> BoxFuture<'a, Result<OneShotOutput>> {
        Box::pin(self.run_job(job))
    }

    fn project_containers<'a>(
        &'a self,
        project: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ServiceContainer>>> {
        Box::pin(self.list_project(project))
    }

    fn components(&self) -> BoxFuture<'_, Result<Vec<Component>>> {
        Box::pin(self.list_components())
    }

    fn stop<'a>(&'a self, container: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.stop_container(container))
    }

    fn remove_container<'a>(&'a self, container: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.engine
                .remove_container(container)
                .await
                .map_err(engine_error)
        })
    }

    fn compose_up<'a>(
        &'a self,
        install_dir: &'a Path,
        wait: Duration,
    ) -> BoxFuture<'a, Result<()>> {
        let args = vec![
            "-f".to_owned(),
            install_dir.join("compose.yaml").display().to_string(),
            "up".to_owned(),
            "-d".to_owned(),
            "--wait".to_owned(),
            "--wait-timeout".to_owned(),
            wait.as_secs().to_string(),
            "--remove-orphans".to_owned(),
        ];
        Box::pin(async move { self.compose(&args, wait + COMPOSE_GRACE).await })
    }

    fn compose_down<'a>(&'a self, project: &'a str) -> BoxFuture<'a, Result<()>> {
        let args = vec!["-p".to_owned(), project.to_owned(), "down".to_owned()];
        Box::pin(async move { self.compose(&args, COMPOSE_DOWN).await })
    }

    fn container<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Option<ContainerInfo>>> {
        Box::pin(async move {
            let Some(info) = self.inspect(name).await? else {
                return Ok(None);
            };
            let state = info.state.unwrap_or_default();
            let policy = info
                .host_config
                .and_then(|h| h.restart_policy)
                .and_then(|p| p.name);
            Ok(Some(ContainerInfo {
                image_id: info.image.unwrap_or_default(),
                config_image: info.config.and_then(|c| c.image).unwrap_or_default(),
                running: state.running.unwrap_or(false) || state.restarting.unwrap_or(false),
                restart_disabled: matches!(
                    policy,
                    None | Some(RestartPolicyNameEnum::EMPTY | RestartPolicyNameEnum::NO)
                ),
            }))
        })
    }

    fn disable_restart<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let body = ContainerUpdateBody {
                restart_policy: Some(RestartPolicy {
                    name: Some(RestartPolicyNameEnum::NO),
                    maximum_retry_count: None,
                }),
                ..Default::default()
            };
            self.engine
                .docker
                .update_container(name, body)
                .await
                .map_err(|e| failed(bollard_message(e)))
        })
    }

    fn start_updater<'a>(&'a self, spec: &'a UpdaterSpec) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.create_updater(spec))
    }
}
