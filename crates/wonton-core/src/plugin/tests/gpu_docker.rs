//! 真实 Docker：显卡设备真的进了它该进的服务（ADR-0018）。
//!
//! 假 Docker 只能证明请求里写了什么；这里让 Docker 真建容器，再从容器里打开设备：AMD 宿主交的是 `/dev/dri`，
//! 有 KFD 时还有 `/dev/kfd`，容器带着宿主设备节点的 GID，非 root 的用户也打得开；没列在 `gpu.services` 的服务
//! 什么都不拿。没有显卡的宿主用空 sysfs 加空 `/dev` 冒充：`required` 装不上并说清缺什么，`optional` 装得上、
//! 不给设备。

use std::time::Duration;

use super::super::Permission;
use super::super::PluginStatus;
use super::super::docker::Engine;
use super::super::docker_exec::ExecOutput;
use super::super::gpu::{HostPaths, probe};
use super::super::requirements::GpuVendor;
use super::manager::manager_in_instance;
use super::package;
use crate::test_docker::Names;

use std::os::unix::fs::MetadataExt;

/// 本地从 ECR Public 拉；CI 经 `WONTON_TEST_IMAGES` 指到镜像仓库副本（wonton-test-images）。
fn image() -> String {
    wonton_test_images::image(
        "public.ecr.aws/docker/library/busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    )
}

/// 共用主机上 Docker 建容器偶尔超过一分钟；这是安装和启动的期限（普通请求用 bollard 自己的 120 秒）。
const INSTALL_TIMEOUT: Duration = crate::config::DEFAULT_INSTALL_TIMEOUT;

/// 只有 `comfy` 要显卡，`web` 不要；都以和核心相同的非 root 用户运行，设备所属的组得靠 GroupAdd 给。
fn compose(need: &str) -> String {
    let image = image();
    package(
        &format!(
            "  comfy:\n    image: {image}\n    user: '${{PUID}}:${{PGID}}'\n    command: ['sleep', '300']\n  \
             web:\n    image: {image}\n    user: '${{PUID}}:${{PGID}}'\n    command: ['sleep', '300']"
        ),
        &format!(
            "  gpu: {{services: [comfy]}}\n  permissions: [devices.gpu]\n  requirements: {{gpu: {{need: {need}}}}}"
        ),
    )
}

/// 空 sysfs 和空 `/dev` 冒充没有显卡的宿主，Docker 是真的。
fn empty_host(dir: &tempfile::TempDir) -> HostPaths {
    let paths = HostPaths {
        dev: Some(dir.path().join("dev")),
        udev: None,
        sys: dir.path().join("sys"),
        proc: dir.path().join("proc"),
    };
    std::fs::create_dir_all(paths.dev.as_ref().unwrap()).unwrap();
    std::fs::create_dir_all(&paths.sys).unwrap();
    std::fs::create_dir_all(&paths.proc).unwrap();
    paths
}

fn real_host() -> HostPaths {
    HostPaths::new(Some("/dev".into()), None)
}

fn granted() -> Permission {
    "devices.gpu".parse().unwrap()
}

#[tokio::test]
#[ignore = "需要 Docker 和显卡"]
async fn docker_hands_render_nodes_and_their_groups_to_declared_services_only() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    let Ok(probed) = probe(&real_host(), &super::langs()) else {
        println!("跳过：这台主机没有可用的显卡");
        return;
    };
    assert!(
        probed.vendors.contains(&GpuVendor::Amd),
        "要 AMD 的渲染节点：{:?}",
        probed.vendors
    );

    crate::test_docker::ensure_image(&engine, &image()).await;
    let names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    let mut manager =
        manager_in_instance(engine.clone(), &dir, names.name()).with_host(real_host());
    manager.cfg.install_timeout = INSTALL_TIMEOUT;
    let m = manager;
    m.install(&compose("required"), &[granted()], super::langs())
        .await
        .unwrap();
    assert_eq!(m.list()[0].status, PluginStatus::Running);

    let comfy = names.container("app-test", "comfy");
    let inspected = engine.inspect_container(&comfy).await.unwrap().unwrap();
    let host = inspected.host_config.unwrap();
    let devices: Vec<String> = host
        .devices
        .unwrap_or_default()
        .iter()
        .filter_map(|d| d.path_in_container.clone())
        .collect();
    assert!(devices.contains(&"/dev/dri".to_owned()));
    let gids: Vec<String> = host.group_add.unwrap_or_default();
    let gid = |path: &str| std::fs::metadata(path).unwrap().gid().to_string();
    assert!(gids.contains(&gid("/dev/dri/renderD128")));
    if probed.devices.iter().any(|d| d == "/dev/kfd") {
        assert!(devices.contains(&"/dev/kfd".to_owned()));
        assert!(gids.contains(&gid("/dev/kfd")));
    }

    async fn run(engine: &Engine, container: &str, argv: &[&str]) -> ExecOutput {
        let argv: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
        engine
            .exec_output(container, &argv, None, Duration::from_secs(10))
            .await
            .unwrap()
    }
    // 非 root 的用户经 GroupAdd 进得了设备：列得出来、打得开。
    let ls = run(&engine, &comfy, &["ls", "/dev/dri"]).await;
    assert_eq!(ls.exit_code, 0, "{}", ls.stderr);
    assert!(ls.stdout.contains("renderD128"), "{}", ls.stdout);
    let read = run(&engine, &comfy, &["test", "-r", "/dev/dri/renderD128"]).await;
    assert_eq!(read.exit_code, 0, "/dev/dri/renderD128 读不了");
    let id = run(&engine, &comfy, &["id", "-G"]).await;
    assert_eq!(id.exit_code, 0, "{}", id.stderr);
    for gid in &gids {
        assert!(id.stdout.contains(gid), "id -G 缺组 {gid}：{}", id.stdout);
    }
    // 没列进 gpu.services 的服务什么都不拿。
    let web = engine
        .inspect_container(&names.container("app-test", "web"))
        .await
        .unwrap()
        .unwrap()
        .host_config
        .unwrap();
    assert!(web.devices.as_ref().is_none_or(|d| d.is_empty()));
    assert!(web.group_add.is_none_or(|g| g.is_empty()));
    assert!(web.device_requests.is_none_or(|r| r.is_empty()));
}

#[tokio::test]
#[ignore = "需要 Docker"]
async fn required_plugins_do_not_install_on_a_host_without_a_gpu() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    crate::test_docker::ensure_image(&engine, &image()).await;
    let names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    let host = empty_host(&dir);
    let mut manager = manager_in_instance(engine, &dir, names.name()).with_host(host);
    manager.cfg.install_timeout = INSTALL_TIMEOUT;
    let m = manager;
    let err = m
        .install(&compose("required"), &[granted()], super::langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "gpu_unavailable", "{err}");
    assert!(
        err.to_string()
            .contains(&super::message("plugin-install-gpu-problem-no-device", &[])),
        "{err}"
    );
    assert!(m.list().is_empty(), "装不上的插件不能留在插件表里");
}

#[tokio::test]
#[ignore = "需要 Docker"]
async fn optional_plugins_install_without_devices_on_a_host_without_a_gpu() {
    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    crate::test_docker::ensure_image(&engine, &image()).await;
    let names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    let host = empty_host(&dir);
    let mut manager = manager_in_instance(engine.clone(), &dir, names.name()).with_host(host);
    manager.cfg.install_timeout = INSTALL_TIMEOUT;
    let m = manager;
    m.install(&compose("optional"), &[granted()], super::langs())
        .await
        .unwrap();
    let host = engine
        .inspect_container(&names.container("app-test", "comfy"))
        .await
        .unwrap()
        .unwrap()
        .host_config
        .unwrap();
    assert!(host.devices.as_ref().is_none_or(|d| d.is_empty()));
}
