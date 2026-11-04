//! 插件的要求和这台主机比：满足、不满足（写明缺什么）、查不出来（不拦安装）。

use serde_json::Value;
use wonton_protocol::{RequirementLine, RequirementStatus as Status};

use super::super::fake_docker::FakeDocker;
use super::super::gpu::fake_host::FakeHost;
use super::super::languages::Languages;
use super::super::manifest::PluginId;
use super::super::names::container_name;
use super::super::{Permission, PluginManager, PluginPackage};
use super::manager::manager_with;
use super::package;

const GIB_KIB: u64 = 1024 * 1024;

fn plugin(requirements: &str) -> String {
    package(
        "  comfy:\n    image: example/comfy",
        &format!("  permissions: []\n  requirements: {requirements}\n"),
    )
}

fn gpu_plugin(gpu: &str) -> String {
    package(
        "  comfy:\n    image: example/comfy",
        &format!(
            "  gpu: {{services: [comfy]}}\n  permissions: [devices.gpu]\n  \
             requirements: {{gpu: {gpu}}}\n"
        ),
    )
}

async fn on_host(host: &FakeHost) -> (FakeDocker, PluginManager, tempfile::TempDir) {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir).with_host(host.paths());
    (docker, m, dir)
}

/// 预览里的 `blocked`；装的时候也要是同样的结果。
async fn blocked(m: &PluginManager, compose: &str) -> Option<String> {
    let package = PluginPackage::parse(compose).unwrap();
    m.preview_install(&package, &Languages::default())
        .await
        .unwrap()
        .blocked
}

/// 装上后详情页里的要求，只有一项。
async fn only_check(m: &PluginManager) -> RequirementLine {
    let id = PluginId::parse("app.test").unwrap();
    let detail = m.judged_detail(&id, &Languages::default()).await.unwrap();
    let [line] = detail.requirements.as_slice() else {
        panic!("{:?}", detail.requirements);
    };
    line.clone()
}

fn gpu_permission() -> Vec<Permission> {
    vec!["devices.gpu".parse().unwrap()]
}

fn host_config(docker: &FakeDocker) -> Value {
    docker
        .created(&container_name("app-test", "comfy"))
        .expect("容器已创建")["HostConfig"]
        .clone()
}

#[tokio::test]
async fn too_little_memory_is_refused_as_unsupported_and_says_how_much_there_is() {
    let host = FakeHost::empty();
    host.memory_total_kib(GIB_KIB * 3 / 2);
    let (docker, m, _dir) = on_host(&host).await;
    let compose = plugin("{memory: 2G}");

    let reason = blocked(&m, &compose).await.unwrap();
    assert!(reason.contains("这台主机只有 1.5 GB 内存"), "{reason}");
    let err = m.install(&compose, &[]).await.unwrap_err();
    assert_eq!(err.kind(), "unsupported", "{err}");
    assert!(err.to_string().contains("1.5 GB"), "{err}");
    assert!(docker.containers().is_empty());
}

#[tokio::test]
async fn memory_a_little_under_the_nominal_size_is_enough() {
    let host = FakeHost::empty();
    host.memory_total_kib(GIB_KIB * 19 / 10);
    let (_docker, m, _dir) = on_host(&host).await;
    let compose = plugin("{memory: 2G}");

    assert_eq!(blocked(&m, &compose).await, None);
    m.install(&compose, &[]).await.unwrap();
    let line = only_check(&m).await;
    assert_eq!((line.status, line.problem), (Status::Met, None));
}

#[tokio::test]
async fn memory_that_cannot_be_read_is_reported_and_does_not_block() {
    let host = FakeHost::empty();
    let (_docker, m, _dir) = on_host(&host).await;
    let compose = plugin("{memory: 2G}");

    assert_eq!(blocked(&m, &compose).await, None);
    m.install(&compose, &[]).await.unwrap();
    let line = only_check(&m).await;
    assert_eq!(line.status, Status::Unknown);
    assert!(line.problem.unwrap().contains("meminfo"));
}

#[tokio::test]
async fn the_wrong_arch_is_refused_as_unsupported() {
    let host = FakeHost::empty();
    let (_docker, m, _dir) = on_host(&host).await;
    let compose = plugin("{arch: [arm64]}");

    let err = m.install(&compose, &[]).await.unwrap_err();
    assert_eq!(err.kind(), "unsupported", "{err}");
    assert!(err.to_string().contains("x86_64"), "{err}");
    assert!(blocked(&m, &compose).await.unwrap().contains("x86_64"));
    m.install(&plugin("{arch: [amd64]}"), &[]).await.unwrap();
    assert_eq!(only_check(&m).await.status, Status::Met);
}

#[tokio::test]
async fn a_card_from_the_wrong_vendor_is_refused_or_left_out() {
    let host = FakeHost::amd();
    let (docker, m, _dir) = on_host(&host).await;

    let compose = gpu_plugin("{vendors: [nvidia]}");
    let reason = blocked(&m, &compose).await.unwrap();
    assert!(reason.contains("这台主机没有 NVIDIA 显卡"), "{reason}");
    let err = m.install(&compose, &gpu_permission()).await.unwrap_err();
    assert_eq!(err.kind(), "gpu_unavailable", "{err}");

    let compose = gpu_plugin("{need: optional, vendors: [nvidia]}");
    assert_eq!(blocked(&m, &compose).await, None);
    m.install(&compose, &gpu_permission()).await.unwrap();
    assert_eq!(only_check(&m).await.status, Status::Degraded);
    assert_eq!(host_config(&docker)["Devices"], Value::Null);
}

#[tokio::test]
async fn a_host_with_two_vendors_hands_over_only_the_wanted_one() {
    let host = FakeHost::amd();
    host.pci("0000:01:00.0", "0x10de", "0x030000", Some("nvidia"));
    std::fs::create_dir_all(host.sys().join("module/nvidia")).unwrap();
    let (docker, m, _dir) = on_host(&host).await;
    docker.set_runtimes(&["runc", "nvidia"]);

    m.install(&gpu_plugin("{vendors: [nvidia]}"), &gpu_permission())
        .await
        .unwrap();
    let config = host_config(&docker);
    assert_eq!(config["Devices"], Value::Null, "{config}");
    assert!(config["DeviceRequests"].is_array(), "{config}");
    m.uninstall(&PluginId::parse("app.test").unwrap(), true)
        .await
        .unwrap();

    // 没装 Toolkit 也不影响要 AMD 的插件。
    docker.set_runtimes(&["runc"]);
    m.install(&gpu_plugin("{vendors: [amd]}"), &gpu_permission())
        .await
        .unwrap();
    let config = host_config(&docker);
    assert_eq!(config["DeviceRequests"], Value::Null, "{config}");
    assert!(config["Devices"].is_array(), "{config}");
}

#[tokio::test]
async fn video_memory_counts_on_a_discrete_card_but_not_on_an_integrated_one() {
    let host = FakeHost::amd();
    let device = host.sys().join("bus/pci/devices/0000:05:00.0");
    std::fs::write(device.join("mem_info_vram_total"), "4294967296\n").unwrap();
    let (_docker, m, _dir) = on_host(&host).await;
    let compose = gpu_plugin("{memory: 8G}");

    // 集成显卡：划给它的 4G 不是上限。
    assert_eq!(blocked(&m, &compose).await, None);
    m.install(&compose, &gpu_permission()).await.unwrap();
    let line = only_check(&m).await;
    assert_eq!(line.status, Status::Unknown);
    assert!(line.problem.unwrap().contains("4 GB"));
    m.uninstall(&PluginId::parse("app.test").unwrap(), true)
        .await
        .unwrap();

    std::fs::write(device.join("mem_info_vram_vendor"), "samsung\n").unwrap();
    let reason = blocked(&m, &compose).await.unwrap();
    assert!(reason.contains("显存只有 4 GB，要 8G"), "{reason}");
    let err = m.install(&compose, &gpu_permission()).await.unwrap_err();
    assert_eq!(err.kind(), "gpu_unavailable", "{err}");

    assert_eq!(blocked(&m, &gpu_plugin("{memory: 4G}")).await, None);
}

#[tokio::test]
async fn nvidia_memory_that_nobody_can_ask_for_does_not_block() {
    let host = FakeHost::nvidia();
    let (docker, m, _dir) = on_host(&host).await;
    docker.set_runtimes(&["runc", "nvidia"]);
    let compose = gpu_plugin("{memory: 8G}");

    assert_eq!(blocked(&m, &compose).await, None);
    m.install(&compose, &gpu_permission()).await.unwrap();
    assert_eq!(only_check(&m).await.status, Status::Unknown);
    assert!(host_config(&docker)["DeviceRequests"].is_array());
}
