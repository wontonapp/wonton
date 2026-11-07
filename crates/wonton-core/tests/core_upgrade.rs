//! 升级器对真的 Docker 和 compose：换镜像、等健康、失败回滚（ADR-0025）。假主机测不出 `up --wait` 和重启计数这些 Engine
//! 的行为。
//!
//! 用 busybox 做的假核心：`init` 写 compose 文件，`serve` 只是睡觉。项目名每次随机，CI 的 runner 共用一个 Docker，
//! 几个测试同时跑也互不干扰，也不碰这台主机上真装的云吞。

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use futures_util::future::BoxFuture;
use wonton_core::plugin::docker::Engine;
use wonton_core::upgrade::docker_host::DockerHost;
use wonton_core::upgrade::host::{
    Component, ContainerInfo, Host, ImageInfo, OneShot, OneShotOutput, Progress, Result,
    ServiceContainer, UpdaterSpec,
};
use wonton_core::upgrade::state::{self, ImageRef, Layout, Outcome, UpgradeState};
use wonton_core::upgrade::{PROJECT, updater};
use wonton_protocol::CoreUpgradeFailure;

/// npchy 和 CI 的 runner 连不上 Docker Hub，从 ECR Public 拉。
const BASE: &str = "public.ecr.aws/docker/library/busybox:1.37";
const OBSERVE: Duration = Duration::from_secs(8);

const FAKE_CORE: &str = r#"#!/bin/sh
case "$1" in
  init)
    cp /fake/compose.yaml "$2/compose.yaml"
    echo "$FAKE_VERSION" >"$2/written-by"
    # 测试不是 root，备份时要保留属主。
    chown "$(stat -c %u:%g "$2")" "$2/compose.yaml" "$2/written-by"
    ;;
  compose-images) ;;
  healthcheck) [ "$FAKE_HEALTHY" = 1 ] ;;
  serve)
    # 1 号进程不响应 SIGTERM，不接住的话每次 stop 都要等满 30 秒才被 SIGKILL。
    trap 'exit 0' TERM
    [ -z "$FAKE_CRASH_AFTER" ] || { sleep "$FAKE_CRASH_AFTER" & wait; exit 1; }
    sleep 3600 & wait
    ;;
  *) echo "认不出 $1" >&2; exit 2 ;;
esac
"#;

/// 真的 Docker，只把会碰到这台主机上别的东西的几件事隔开：系统服务的项目名和一次性容器的名字是常量（换成带前缀的），
/// 别的组件和升级器容器都是整台主机只有一份的（这里没有）。
struct Isolated {
    inner: DockerHost,
    project: String,
}

impl Host for Isolated {
    fn now(&self) -> u64 {
        self.inner.now()
    }
    fn sleep(&self, duration: Duration) -> BoxFuture<'_, ()> {
        self.inner.sleep(duration)
    }
    fn free_bytes(&self, path: &Path) -> Result<u64> {
        self.inner.free_bytes(path)
    }
    fn image<'a>(&'a self, reference: &'a str) -> BoxFuture<'a, Result<Option<ImageInfo>>> {
        self.inner.image(reference)
    }
    fn registry_digest<'a>(&'a self, source: &'a str) -> BoxFuture<'a, Result<String>> {
        self.inner.registry_digest(source)
    }
    fn pull<'a>(
        &'a self,
        image: &'a str,
        progress: &'a (dyn Fn(Progress) + Send + Sync),
    ) -> BoxFuture<'a, Result<()>> {
        self.inner.pull(image, progress)
    }
    fn remove_image<'a>(&'a self, image: &'a str) -> BoxFuture<'a, Result<()>> {
        self.inner.remove_image(image)
    }
    fn run_once<'a>(&'a self, job: &'a OneShot) -> BoxFuture<'a, Result<OneShotOutput>> {
        let job = OneShot {
            name: format!("{}-{}", self.project, job.name),
            ..job.clone()
        };
        Box::pin(async move { self.inner.run_once(&job).await })
    }
    fn project_containers<'a>(
        &'a self,
        project: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ServiceContainer>>> {
        assert_eq!(project, PROJECT);
        self.inner.project_containers(&self.project)
    }
    fn components(&self) -> BoxFuture<'_, Result<Vec<Component>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn stop<'a>(&'a self, container: &'a str) -> BoxFuture<'a, Result<()>> {
        self.inner.stop(container)
    }
    fn remove_container<'a>(&'a self, container: &'a str) -> BoxFuture<'a, Result<()>> {
        self.inner.remove_container(container)
    }
    fn compose_up<'a>(
        &'a self,
        install_dir: &'a Path,
        wait: Duration,
    ) -> BoxFuture<'a, Result<()>> {
        self.inner.compose_up(install_dir, wait)
    }
    fn compose_down<'a>(&'a self, project: &'a str) -> BoxFuture<'a, Result<()>> {
        self.inner.compose_down(project)
    }
    fn container<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Option<ContainerInfo>>> {
        self.inner.container(name)
    }
    fn disable_restart<'a>(&'a self, _name: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn start_updater<'a>(&'a self, spec: &'a UpdaterSpec) -> BoxFuture<'a, Result<()>> {
        self.inner.start_updater(spec)
    }
}

fn docker(args: &[&str]) -> std::process::Output {
    Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("执行 docker")
}

fn must(args: &[&str]) -> String {
    let output = docker(args);
    assert!(
        output.status.success(),
        "docker {args:?}：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// 一套假的安装：安装目录、数据目录和带前缀的项目，用完删掉。
struct Install {
    project: String,
    dir: tempfile::TempDir,
    images: Vec<String>,
}

impl Drop for Install {
    fn drop(&mut self) {
        let _ = docker(&["compose", "-p", &self.project, "down", "-t", "1", "-v"]);
        for image in &self.images {
            let _ = docker(&["rmi", "-f", image]);
        }
    }
}

impl Install {
    fn new(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        // 容器里的 root 要能写，测试结束后这边要能删。
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let id = &wonton_core::secret::random_token()[..12];
        let project = format!("core-upgrade-test-{id}-{name}");
        let install = Self {
            project,
            dir,
            images: Vec::new(),
        };
        fs::create_dir_all(install.layout().install_dir).unwrap();
        fs::create_dir_all(install.layout().core_dir()).unwrap();
        install
    }

    fn layout(&self) -> Layout {
        Layout {
            install_dir: self.dir.path().join("install"),
            data_dir: self.dir.path().join("data"),
        }
    }

    fn compose_wrapper(&self) -> PathBuf {
        let path = self.dir.path().join("compose");
        fs::write(&path, "#!/bin/sh\nexec docker compose \"$@\"\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// `crash_after` 为空时一直活着；不为空时先健康，过这么多秒后崩溃。
    fn build(&mut self, version: &str, healthy: bool, crash_after: &str) -> ImageRef {
        let context = tempfile::tempdir().unwrap();
        let script = context.path().join("wonton-core");
        fs::write(&script, FAKE_CORE).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let compose = format!(
            "name: {}\nservices:\n  core:\n    image: ${{WONTON_CORE_IMAGE:?}}\n    restart: unless-stopped\n    \
             stop_grace_period: 1s\n    healthcheck:\n      test: [\"CMD\", \"wonton-core\", \"healthcheck\"]\n      \
             interval: 1s\n      timeout: 3s\n      retries: 2\n",
            self.project
        );
        fs::write(context.path().join("compose.yaml"), compose).unwrap();
        let healthy = u8::from(healthy);
        let dockerfile = format!(
            "FROM {BASE}\nCOPY wonton-core /bin/wonton-core\nCOPY compose.yaml /fake/compose.yaml\n\
             ENV FAKE_VERSION={version} FAKE_HEALTHY={healthy} FAKE_CRASH_AFTER={crash_after}\n\
             LABEL org.opencontainers.image.version={version}\n\
             ENTRYPOINT [\"wonton-core\"]\nCMD [\"serve\"]\n"
        );
        fs::write(context.path().join("Dockerfile"), dockerfile).unwrap();
        let image = format!("{}:{version}", self.project);
        must(&[
            "build",
            "-q",
            "-t",
            &image,
            &context.path().display().to_string(),
        ]);
        self.images.push(image.clone());
        ImageRef {
            image,
            digest: Some(format!("sha256:{version}")),
            version: Some(version.to_owned()),
        }
    }

    /// 按安装脚本的样子装好 `from`，并写下一份核心数据。
    fn install(&self, from: &ImageRef) -> Isolated {
        let layout = self.layout();
        let install_dir = layout.install_dir.display().to_string();
        must(&[
            "run",
            "--rm",
            "-v",
            &format!("{install_dir}:{install_dir}"),
            &from.image,
            "init",
            &install_dir,
            "wonton.localhost",
        ]);
        fs::write(
            layout.env_path(),
            format!(
                "WONTON_CORE_SOURCE={}\nWONTON_CORE_IMAGE={}\nWONTON_INSTALL_DIR={install_dir}\n\
                 WONTON_DATA_DIR={}\nWONTON_PORT=1\nWONTON_TUNNEL_PORT=2\nWONTON_BASE_DOMAIN=wonton.localhost\n",
                from.image,
                from.image,
                layout.data_dir.display()
            ),
        )
        .unwrap();
        fs::write(layout.core_dir().join("data"), "旧版本写的").unwrap();
        must(&[
            "compose",
            "-f",
            &format!("{install_dir}/compose.yaml"),
            "up",
            "-d",
            "--wait",
            "--wait-timeout",
            "60",
        ]);
        Isolated {
            inner: DockerHost::with_compose(
                Engine::connect().unwrap(),
                self.compose_wrapper(),
                Box::new({
                    let data_dir = layout.data_dir.clone();
                    move || wonton_core::plugin::store::read_owner_languages(&data_dir)
                }),
            ),
            project: self.project.clone(),
        }
    }

    /// 从 `from` 升到 `to`，返回写下的最终状态。
    async fn upgrade(&self, host: &Isolated, from: &ImageRef, to: &ImageRef) -> UpgradeState {
        let layout = self.layout();
        let langs = wonton_core::plugin::languages::Languages::from_header(None);
        let started = UpgradeState::new(
            "test".to_owned(),
            from.clone(),
            to.clone(),
            Vec::new(),
            host.now(),
        );
        state::save(&layout.state_path(), &started, &langs).unwrap();
        updater::run(host, layout.clone(), langs.clone(), OBSERVE)
            .await
            .unwrap();
        state::load(&layout.state_path(), &langs).unwrap().unwrap()
    }

    async fn core_image(&self, host: &Isolated) -> String {
        let containers = host.project_containers(PROJECT).await.unwrap();
        let core = containers.iter().find(|c| c.service == "core").unwrap();
        assert_eq!(core.status, "running", "{core:?}");
        core.image.clone()
    }

    fn env_image(&self) -> String {
        let env = fs::read_to_string(self.layout().env_path()).unwrap();
        env.lines()
            .find_map(|l| l.strip_prefix("WONTON_CORE_IMAGE="))
            .unwrap()
            .to_owned()
    }

    fn written_by(&self) -> String {
        let path = self.layout().install_dir.join("written-by");
        fs::read_to_string(path).unwrap().trim().to_owned()
    }
}

fn reason(state: &UpgradeState) -> Option<CoreUpgradeFailure> {
    state.problem.as_ref().map(|p| p.reason)
}

#[tokio::test]
#[ignore = "需要 Docker，起几个一次性的容器"]
async fn upgrading_replaces_the_core_and_keeps_a_backup() {
    let mut install = Install::new("ok");
    let from = install.build("old", true, "");
    let to = install.build("new", true, "");
    let host = install.install(&from);

    let ended = install.upgrade(&host, &from, &to).await;

    assert_eq!(
        ended.result,
        Some(Outcome::Completed),
        "{:?}",
        ended.problem
    );
    assert_eq!(install.core_image(&host).await, to.image);
    assert_eq!(install.env_image(), to.image);
    assert_eq!(install.written_by(), "new");
    let backup = ended.backup_dir.expect("备份目录");
    assert!(backup.join(".complete").exists());
}

#[tokio::test]
#[ignore = "需要 Docker，起几个一次性的容器"]
async fn a_new_version_that_never_gets_healthy_is_rolled_back() {
    let mut install = Install::new("unhealthy");
    let from = install.build("old", true, "");
    let to = install.build("bad", false, "");
    let host = install.install(&from);

    let ended = install.upgrade(&host, &from, &to).await;

    assert_eq!(
        ended.result,
        Some(Outcome::RolledBack),
        "{:?}",
        ended.problem
    );
    assert_eq!(reason(&ended), Some(CoreUpgradeFailure::Unhealthy));
    assert_eq!(install.core_image(&host).await, from.image);
    assert_eq!(install.env_image(), from.image);
    assert_eq!(install.written_by(), "old");
    let data = fs::read_to_string(install.layout().core_dir().join("data")).unwrap();
    assert_eq!(data, "旧版本写的");
}

/// `up --wait` 通过之后才崩的版本，靠观察阶段的重启计数发现。
#[tokio::test]
#[ignore = "需要 Docker，起几个一次性的容器"]
async fn a_new_version_that_crashes_while_observed_is_rolled_back() {
    let mut install = Install::new("crash");
    let from = install.build("old", true, "");
    let to = install.build("crash", true, "4");
    let host = install.install(&from);

    let ended = install.upgrade(&host, &from, &to).await;

    assert_eq!(
        ended.result,
        Some(Outcome::RolledBack),
        "{:?}",
        ended.problem
    );
    assert_eq!(reason(&ended), Some(CoreUpgradeFailure::Crashed));
    assert_eq!(install.core_image(&host).await, from.image);
    assert_eq!(install.env_image(), from.image);
}
