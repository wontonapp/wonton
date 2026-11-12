//! 测试用的假主机：文件系统是真的（临时目录），Docker、compose 和时钟是假的，记下做过的操作。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use futures_util::future::{self, BoxFuture};
use tokio::sync::Notify;

use super::host::{
    Component, ContainerInfo, Host, HostError, ImageInfo, OneShot, OneShotOutput, Progress, Result,
    ServiceContainer, UpdaterSpec,
};
use super::state::Layout;
use super::{CORE_SERVICE, LABEL_COMPONENTS, UPDATER_CONTAINER};

pub const OLD: &str = "reg/core@sha256:old";
pub const NEW: &str = "reg/core@sha256:new";

#[derive(Default)]
pub struct World {
    pub images: HashMap<String, ImageInfo>,
    pub registry: HashMap<String, String>,
    /// 这些镜像拉不到，值是原因。
    pub pull_errors: HashMap<String, String>,
    /// 拉到时才出现的镜像（模拟仓库里有）。
    pub pullable: HashMap<String, ImageInfo>,
    /// 这些核心镜像 `up` 时过不了健康检查。
    pub unhealthy: HashSet<String>,
    /// 这些核心镜像过了健康检查，观察时崩。
    pub crashes: HashSet<String>,
    /// 这些镜像的 `init` 失败，值是标准错误。
    pub init_errors: HashMap<String, String>,
    /// 这样的 `up` 第几次失败：回滚也失败的场景用。
    pub up_error: Option<String>,
    pub containers: Vec<ServiceContainer>,
    pub components: Vec<Component>,
    pub updater: Option<ContainerInfo>,
    pub updater_start_error: Option<String>,
    pub updater_spec: Option<UpdaterSpec>,
    pub free: u64,
    pub log: Vec<String>,
    /// 走到这个操作时停住（只停一次）：测试丢掉 future，模拟升级器被杀。
    pub stall_at: Option<String>,
    /// 走到这个操作时，把取消标记写上：模拟主人在这一刻点了取消。
    pub cancel_at: Option<(String, PathBuf)>,
    /// 调用 `registry_digest` 时要报的错。
    pub registry_error: Option<String>,
}

pub struct FakeHost {
    pub world: Mutex<World>,
    pub now: AtomicU64,
    pub stalled: Notify,
    install_dir: PathBuf,
}

fn core_container(image: &str, image_id: &str) -> ServiceContainer {
    ServiceContainer {
        id: format!("id-{image}"),
        name: "wonton-core".into(),
        service: CORE_SERVICE.into(),
        image: image.into(),
        image_id: image_id.into(),
        status: "running".into(),
        health: Some("healthy".into()),
        long_running: true,
        ..Default::default()
    }
}

impl FakeHost {
    /// 一台正式安装的主机：跑着 [`OLD`]，本机已有 [`NEW`] 和其他服务的镜像。
    pub fn installed(layout: &Layout) -> Self {
        let image = |version: &str| ImageInfo {
            id: format!("id-{version}"),
            repo_digests: Vec::new(),
            labels: HashMap::from([(super::LABEL_VERSION.to_owned(), version.to_owned())]),
            size: 100,
        };
        let world = World {
            images: HashMap::from([
                (OLD.to_owned(), image("0.1.0")),
                ("id-core".to_owned(), image("0.1.0")),
                (NEW.to_owned(), image("0.2.0")),
                ("hydra:1".to_owned(), image("1")),
            ]),
            containers: vec![
                core_container(OLD, "id-0.1.0"),
                ServiceContainer {
                    id: "id-hydra-migrate".into(),
                    name: "wonton-hydra-migrate-1".into(),
                    service: "hydra-migrate".into(),
                    image: "hydra:1".into(),
                    status: "exited".into(),
                    ..Default::default()
                },
                ServiceContainer {
                    id: "id-tuwunel".into(),
                    name: "wonton-tuwunel-1".into(),
                    service: "tuwunel".into(),
                    image: "tuwunel:1".into(),
                    status: "running".into(),
                    health: Some("healthy".into()),
                    long_running: true,
                    volumes: vec!["wonton_tuwunel".into()],
                    ..Default::default()
                },
            ],
            free: u64::MAX / 2,
            ..Default::default()
        };
        Self {
            world: Mutex::new(world),
            now: AtomicU64::new(1_000_000),
            stalled: Notify::new(),
            install_dir: layout.install_dir.clone(),
        }
    }

    pub fn world(&self) -> MutexGuard<'_, World> {
        self.world.lock().unwrap()
    }

    pub fn log(&self) -> Vec<String> {
        self.world().log.clone()
    }

    pub fn did(&self, op: &str) -> bool {
        self.log().iter().any(|l| l == op)
    }

    pub fn advance(&self, duration: Duration) {
        self.now.fetch_add(duration.as_secs(), Ordering::SeqCst);
    }

    pub fn core_image(&self) -> String {
        let world = self.world();
        let core = world.containers.iter().find(|c| c.service == CORE_SERVICE);
        core.map(|c| c.image.clone()).unwrap_or_default()
    }

    /// 记下操作；走到要停的地方就永远不返回。
    async fn op(&self, name: String) {
        let stall = {
            let mut world = self.world();
            world.log.push(name.clone());
            if let Some((at, path)) = &world.cancel_at
                && *at == name
            {
                fs::write(path, b"").unwrap();
            }
            let stall = world.stall_at.as_deref() == Some(name.as_str());
            if stall {
                world.stall_at = None;
            }
            stall
        };
        if stall {
            self.stalled.notify_one();
            future::pending::<()>().await;
        }
    }

    fn env_image(&self) -> String {
        let text = fs::read_to_string(self.install_dir.join(".env")).unwrap_or_default();
        text.lines()
            .rev()
            .find_map(|l| l.strip_prefix("WONTON_CORE_IMAGE="))
            .unwrap_or_default()
            .to_owned()
    }

    /// `init` 改写安装目录：回滚要能把这些退掉。
    fn init(&self, image: &str) -> OneShotOutput {
        if let Some(stderr) = self.world().init_errors.get(image) {
            return output(1, "", stderr);
        }
        fs::write(
            self.install_dir.join("compose.yaml"),
            format!("compose of {image}"),
        )
        .unwrap();
        output(0, "", "")
    }
}

fn output(exit_code: i64, stdout: &str, stderr: &str) -> OneShotOutput {
    OneShotOutput {
        exit_code,
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
    }
}

impl Host for FakeHost {
    fn now(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }

    fn sleep(&self, duration: Duration) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.advance(duration);
            tokio::task::yield_now().await;
            // 观察时模拟崩溃：第一次睡醒后重启次数变了。
            let mut world = self.world();
            let crashes = world.crashes.clone();
            for container in &mut world.containers {
                if crashes.contains(&container.image) {
                    container.restart_count += 1;
                }
            }
        })
    }

    fn free_bytes(&self, _path: &Path) -> Result<u64> {
        Ok(self.world().free)
    }

    fn image<'a>(&'a self, reference: &'a str) -> BoxFuture<'a, Result<Option<ImageInfo>>> {
        Box::pin(async move { Ok(self.world().images.get(reference).cloned()) })
    }

    fn registry_digest<'a>(&'a self, source: &'a str) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let world = self.world();
            if let Some(e) = &world.registry_error {
                return Err(HostError::Failed(e.clone()));
            }
            let found = world.registry.get(source).cloned();
            found.ok_or_else(|| HostError::NotFound(format!("仓库里没有 {source}")))
        })
    }

    fn pull<'a>(
        &'a self,
        image: &'a str,
        progress: &'a (dyn Fn(Progress) + Send + Sync),
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.op(format!("pull {image}")).await;
            let mut world = self.world();
            if let Some(e) = world.pull_errors.get(image) {
                return Err(HostError::Failed(e.clone()));
            }
            progress(Progress { done: 5, total: 10 });
            let info = world.pullable.get(image).cloned().unwrap_or_default();
            world.images.insert(image.to_owned(), info);
            Ok(())
        })
    }

    fn remove_image<'a>(&'a self, image: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.op(format!("remove-image {image}")).await;
            self.world().images.remove(image);
            Ok(())
        })
    }

    fn run_once<'a>(&'a self, job: &'a OneShot) -> BoxFuture<'a, Result<OneShotOutput>> {
        Box::pin(async move {
            self.op(format!("run {} {}", job.name, job.image)).await;
            Ok(match job.name.as_str() {
                "wonton-updater-images" => output(0, "hydra:1\ntuwunel:1\n", ""),
                "wonton-updater-init" => self.init(&job.image),
                "wonton-updater-volume-size" => output(0, "1000\t/volume\n", ""),
                _ => output(0, "", ""),
            })
        })
    }

    fn project_containers<'a>(
        &'a self,
        _project: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ServiceContainer>>> {
        Box::pin(async move { Ok(self.world().containers.clone()) })
    }

    fn components(&self) -> BoxFuture<'_, Result<Vec<Component>>> {
        Box::pin(async move { Ok(self.world().components.clone()) })
    }

    fn stop<'a>(&'a self, container: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.op(format!("stop {container}")).await;
            let mut world = self.world();
            if container == UPDATER_CONTAINER
                && let Some(updater) = &mut world.updater
            {
                updater.running = false;
            }
            for found in world.containers.iter_mut().filter(|c| c.name == container) {
                found.status = "exited".into();
            }
            Ok(())
        })
    }

    fn remove_container<'a>(&'a self, container: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.op(format!("remove-container {container}")).await;
            if container == UPDATER_CONTAINER {
                self.world().updater = None;
            }
            Ok(())
        })
    }

    fn compose_up<'a>(
        &'a self,
        _install_dir: &'a Path,
        wait: Duration,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let image = self.env_image();
            self.op(format!("up {image}")).await;
            assert!(
                wait <= Duration::from_secs(600),
                "up 最多等 600 秒：{wait:?}"
            );
            let mut world = self.world();
            if let Some(e) = world.up_error.as_ref().filter(|_| image == OLD) {
                return Err(HostError::Failed(e.clone()));
            }
            let running = world
                .containers
                .iter()
                .position(|c| c.service == CORE_SERVICE);
            if let Some(index) = running {
                let changed = world.containers[index].image != image;
                if changed || world.containers[index].status != "running" {
                    let image_id = world
                        .images
                        .get(&image)
                        .map(|info| info.id.clone())
                        .unwrap_or_default();
                    world.containers[index] = core_container(&image, &image_id);
                }
            }
            for container in &mut world.containers {
                if container.long_running {
                    container.status = "running".into();
                }
            }
            if world.unhealthy.contains(&image) {
                return Err(HostError::Failed(format!("服务 core 不健康（{image}）")));
            }
            Ok(())
        })
    }

    fn compose_down<'a>(&'a self, project: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.op(format!("down {project}")).await;
            Ok(())
        })
    }

    fn container<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Option<ContainerInfo>>> {
        Box::pin(async move {
            if name == UPDATER_CONTAINER {
                return Ok(self.world().updater.clone());
            }
            Ok(Some(ContainerInfo {
                image_id: "id-core".into(),
                config_image: OLD.into(),
                running: true,
                restart_disabled: false,
            }))
        })
    }

    fn disable_restart<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.op(format!("disable-restart {name}")).await;
            if let Some(updater) = &mut self.world().updater {
                updater.restart_disabled = true;
            }
            Ok(())
        })
    }

    fn start_updater<'a>(&'a self, spec: &'a UpdaterSpec) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.op("start-updater".to_owned()).await;
            let mut world = self.world();
            if let Some(e) = &world.updater_start_error {
                return Err(HostError::Failed(e.clone()));
            }
            world.updater_spec = Some(spec.clone());
            world.updater = Some(ContainerInfo {
                image_id: spec.image_id.clone(),
                config_image: spec.image_id.clone(),
                running: true,
                restart_disabled: false,
            });
            Ok(())
        })
    }
}

/// 给镜像加标签。
pub fn label(host: &FakeHost, image: &str, key: &str, value: &str) {
    let mut world = host.world();
    let info = world.images.get_mut(image).unwrap();
    info.labels.insert(key.to_owned(), value.to_owned());
}

/// 声明这个镜像支持的组件。
pub fn supports(host: &FakeHost, image: &str, components: &str) {
    label(host, image, LABEL_COMPONENTS, components);
}
