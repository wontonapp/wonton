//! 显卡（`devices.gpu`）：核心按宿主实际有的显卡把设备交给清单 `gpu.services` 里的服务，不收 compose 里的
//! `devices`；没有可用的显卡时，`required` 的插件装不上、原因写清是哪种，`optional` 的照常装、不给设备。

use std::os::unix::fs::MetadataExt;

use serde_json::{Value, json};
use wonton_protocol::{CardLine, InstallPreview};

use super::super::fake_docker::FakeDocker;
use super::super::gpu::fake_host::FakeHost;
use super::super::gpu::requirement;
use super::super::languages::Languages;
use super::super::manifest::GpuNeed;
use super::super::names::container_name;
use super::super::render::dry_run;
use super::super::{PluginManager, PluginPackage};
use super::manager::{manager_in_container, manager_with};
use super::{example, package};

/// 两个服务，只有 `comfy` 要显卡；它以非 root 用户运行，所以设备所属的组也得给它。
fn gpu_plugin(extra_manifest: &str) -> String {
    package(
        "  comfy:\n    image: example/comfy\n    user: '1000:1000'\n  web:\n    image: nginx",
        &format!("  gpu: {{services: [comfy]}}\n  permissions: [devices.gpu]\n{extra_manifest}"),
    )
}

fn granted() -> Vec<super::super::Permission> {
    vec!["devices.gpu".parse().unwrap()]
}

/// 容器创建请求里的 `HostConfig`。
fn host_config(docker: &FakeDocker, service: &str) -> Value {
    docker
        .created(&container_name("app-test", service))
        .expect("容器已创建")["HostConfig"]
        .clone()
}

/// 假设备节点是普通文件，属于测试进程的组；root 组（0）不加。
fn expected_groups(host: &FakeHost) -> Value {
    let gid = std::fs::metadata(host.dev().join("kfd")).unwrap().gid();
    if gid == 0 {
        Value::Null
    } else {
        json!([gid.to_string()])
    }
}

/// 安装预览，和 App 的确认页看到的一样。
async fn preview(m: &PluginManager, compose: &str) -> InstallPreview {
    let package = PluginPackage::parse(compose).unwrap();
    m.preview_install(&package, &Languages::default())
        .await
        .unwrap()
}

/// 预览里关于显卡的那一行。
fn gpu_notice(preview: &InstallPreview) -> &CardLine {
    preview
        .notices
        .iter()
        .rfind(|n| n.text.contains("显卡"))
        .unwrap_or_else(|| panic!("预览里没有显卡的说明：{:?}", preview.notices))
}

async fn on_host(host: &FakeHost) -> (FakeDocker, PluginManager, tempfile::TempDir) {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir).with_host(host.paths());
    (docker, m, dir)
}

#[tokio::test]
async fn amd_hosts_hand_kfd_dri_and_their_groups_to_the_declared_services_only() {
    let host = FakeHost::amd();
    let (docker, m, _dir) = on_host(&host).await;
    let compose = gpu_plugin("");
    let preview = preview(&m, &compose).await;
    assert_eq!(preview.blocked, None);
    let notice = gpu_notice(&preview);
    assert!(
        notice
            .text
            .starts_with("显卡交给服务 comfy：AMD 显卡：/dev/kfd、/dev/dri"),
        "{notice:?}"
    );
    assert!(notice.emphasis, "高权限的说明要醒目");

    m.install(&compose, &granted()).await.unwrap();
    let comfy = host_config(&docker, "comfy");
    let device = |path: &str| json!({"PathOnHost": path, "PathInContainer": path, "CgroupPermissions": "rwm"});
    assert_eq!(
        comfy["Devices"],
        json!([device("/dev/kfd"), device("/dev/dri")])
    );
    assert_eq!(comfy["GroupAdd"], expected_groups(&host));
    assert_eq!(
        comfy["DeviceRequests"],
        Value::Null,
        "AMD 不经 NVIDIA 的运行时"
    );
    // 没列在 gpu.services 里的服务什么都不拿。
    let web = host_config(&docker, "web");
    assert_eq!(web["Devices"], Value::Null);
    assert_eq!(web["GroupAdd"], Value::Null);
    assert_eq!(web["DeviceRequests"], Value::Null);
}

#[tokio::test]
async fn nvidia_hosts_get_all_gpus_through_the_container_toolkit() {
    let host = FakeHost::nvidia();
    let (docker, m, _dir) = on_host(&host).await;
    docker.set_runtimes(&["runc", "nvidia"]);
    m.install(&gpu_plugin(""), &granted()).await.unwrap();
    let comfy = host_config(&docker, "comfy");
    assert_eq!(
        comfy["DeviceRequests"],
        json!([{"Count": -1, "Capabilities": [["gpu"]]}])
    );
    assert_eq!(comfy["Devices"], Value::Null);
    assert_eq!(comfy["GroupAdd"], Value::Null);
}

#[tokio::test]
async fn required_plugins_do_not_install_without_a_usable_gpu_and_say_which_part_is_missing() {
    // 没有显卡。
    let host = FakeHost::empty();
    let (docker, m, _dir) = on_host(&host).await;
    let compose = gpu_plugin("");
    let preview = preview(&m, &compose).await;
    assert!(
        preview
            .blocked
            .as_deref()
            .is_some_and(|b| b.contains("这台主机没有显卡")),
        "没写 requirements.gpu 时按 required，这台主机上装不了：{preview:?}"
    );
    assert!(gpu_notice(&preview).text.contains("需要显卡"));
    let err = m.install(&compose, &granted()).await.unwrap_err();
    assert_eq!(err.kind(), "gpu_unavailable", "{err}");
    assert!(err.to_string().contains("这台主机没有显卡"), "{err}");
    assert!(docker.containers().is_empty(), "{:?}", docker.containers());
    assert!(docker.networks().is_empty(), "{:?}", docker.networks());
    assert!(m.list().is_empty(), "装不上的插件不能留在插件表里");

    // 有 NVIDIA 显卡和驱动，但 Docker 没有 nvidia 运行时（没装 NVIDIA Container Toolkit）。
    let host = FakeHost::nvidia();
    let (_docker, m, _dir) = on_host(&host).await;
    let err = m.install(&compose, &granted()).await.unwrap_err();
    assert_eq!(err.kind(), "gpu_unavailable", "{err}");
    assert!(err.to_string().contains("nvidia 运行时"), "{err}");

    // 核心在容器里，compose 没有挂载宿主的 /dev：不拿容器自己的 /dev 当宿主的。
    let host = FakeHost::amd();
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_in_container(engine, &dir).with_host(host.paths_without_dev());
    let err = m.install(&compose, &granted()).await.unwrap_err();
    assert_eq!(err.kind(), "gpu_unavailable", "{err}");
    assert!(err.to_string().contains("WONTON_HOST_DEV"), "{err}");
}

#[tokio::test]
async fn optional_plugins_install_without_devices_when_there_is_no_gpu() {
    let host = FakeHost::empty();
    let (docker, m, _dir) = on_host(&host).await;
    let compose = gpu_plugin("  requirements: {gpu: {need: optional}}");
    let preview = preview(&m, &compose).await;
    assert_eq!(preview.blocked, None, "optional 的照常装");
    assert!(gpu_notice(&preview).text.contains("不带显卡运行"));
    m.install(&compose, &granted()).await.unwrap();
    let comfy = host_config(&docker, "comfy");
    assert_eq!(comfy["Devices"], Value::Null);
    assert_eq!(comfy["GroupAdd"], Value::Null);
    assert_eq!(comfy["DeviceRequests"], Value::Null);
}

#[test]
fn gpu_services_permission_and_requirement_are_declared_together() {
    let parse = |services: &str, manifest: &str| {
        PluginPackage::parse(&package(
            &format!("  web:\n    image: nginx\n{services}"),
            manifest,
        ))
    };
    let cases = [
        ("  gpu: {services: [web]}\n  permissions: []", "devices.gpu"),
        ("  permissions: [devices.gpu]", "gpu.services"),
        (
            "  permissions: []\n  requirements: {gpu: {need: optional}}",
            "requirements.gpu",
        ),
        (
            "  gpu: {services: [web, web]}\n  permissions: [devices.gpu]",
            "重复",
        ),
        (
            "  gpu: {services: []}\n  permissions: [devices.gpu]",
            "至少一项",
        ),
        (
            "  gpu: {services: [nope]}\n  permissions: [devices.gpu]",
            "nope",
        ),
    ];
    for (manifest, needle) in cases {
        let err = parse("", manifest).unwrap_err().to_string();
        assert!(err.contains(needle), "{manifest}: {err}");
    }
    let package = parse("    devices: ['/dev/kfd:/dev/kfd']", "  permissions: []").unwrap();
    let err = dry_run(&package).unwrap_err().to_string();
    assert!(err.contains("设备") || err.contains("固定"), "{err}");
    let ok = parse("", "  gpu: {services: [web]}\n  permissions: [devices.gpu]").unwrap();
    assert_eq!(requirement(&ok.manifest), Some(GpuNeed::Required));
    let optional = parse(
        "",
        "  gpu: {services: [web]}\n  permissions: [devices.gpu]\n  requirements: {gpu: {need: optional}}",
    )
    .unwrap();
    assert_eq!(requirement(&optional.manifest), Some(GpuNeed::Optional));
    let none = parse("", "  permissions: []").unwrap();
    assert_eq!(requirement(&none.manifest), None);
}

#[test]
fn example_gpu_plugins_are_valid() {
    let pkg = PluginPackage::parse(include_str!("examples/gpu.yaml")).unwrap();
    assert_eq!(pkg.manifest.gpu.as_ref().unwrap().services, ["comfyui"]);
    dry_run(&pkg).unwrap();

    let check = example("gpu-check");
    assert_eq!(check.manifest.id.as_str(), "app.wonton.gpu-check");
    assert_eq!(requirement(&check.manifest), Some(GpuNeed::Required));
    dry_run(&check).unwrap();
}
