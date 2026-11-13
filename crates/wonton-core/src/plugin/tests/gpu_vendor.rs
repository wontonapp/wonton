//! 按显卡厂商选镜像（ADR-0099 第 2 步）：按 `requirements.gpu.vendors` 的顺序选本机第一家能用的，换这家的镜像、
//! 注入 `WONTON_GPU_VENDOR`、写进安装记录；一家都不行时 `required` 装不上、`optional` 用通用镜像不给设备。

use serde_json::{Value, json};
use wonton_protocol::InstallPreview;

use super::super::fake_docker::FakeDocker;
use super::super::gpu::fake_host::FakeHost;
use super::super::languages::Languages;
use super::super::manifest::PluginId;
use super::super::names::container_name;
use super::super::requirements::GpuVendor;
use super::super::store::OpenedBy;
use super::super::{Permission, PluginManager, PluginPackage};
use super::manager::{manager_in_container, manager_with};
use super::package;

const AMD_IMAGE: &str =
    "example/comfy:rocm@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NVIDIA_IMAGE: &str =
    "example/comfy:cuda@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const GENERIC: &str =
    "example/comfy@sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn zh() -> Languages {
    Languages::from_header(Some("zh-Hans"))
}

fn granted() -> Vec<Permission> {
    vec![Permission::DevicesGpu]
}

/// 装显卡的插件：`comfy` 要显卡并引用厂商变量，`web` 不要。`extra` 接在 `x-wonton` 下。
fn compose(extra: &str) -> String {
    package(
        &format!(
            "  comfy:\n    image: {GENERIC}\n    environment: {{VENDOR: '${{WONTON_GPU_VENDOR:-none}}'}}\n  \
             web:\n    image: nginx"
        ),
        extra,
    )
}

fn with_images(need: &str, vendors: &[&str], images: &str) -> String {
    let vendors = vendors
        .iter()
        .map(|vendor| format!("\"{vendor}\""))
        .collect::<Vec<_>>()
        .join(", ");
    compose(&format!(
        "  gpu: {{services: [comfy], images: {images}}}\n  permissions: [devices.gpu]\n  \
         requirements:\n    gpu: {{need: {need}, vendors: [{vendors}]}}\n"
    ))
}

async fn on(host: &FakeHost, runtimes: &[&str]) -> (FakeDocker, PluginManager, tempfile::TempDir) {
    let (docker, engine) = FakeDocker::start().await;
    docker.set_runtimes(runtimes);
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir).with_host(host.paths());
    (docker, m, dir)
}

async fn preview(m: &PluginManager, source: &str) -> InstallPreview {
    let package = PluginPackage::parse(source, &super::langs()).unwrap();
    m.preview_install(&package, &zh()).await.unwrap()
}

fn image_of(docker: &FakeDocker, service: &str) -> Value {
    docker
        .created(&container_name("app-test", service))
        .unwrap()["Image"]
        .clone()
}

fn env_of(docker: &FakeDocker, service: &str) -> Vec<String> {
    docker
        .created(&container_name("app-test", service))
        .unwrap()["Env"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn the_listed_order_decides_and_the_vendor_image_replaces_the_generic_one() {
    let host = FakeHost::amd();
    let (docker, m, _dir) = on(&host, &["runc", "nvidia"]).await;
    let source = with_images(
        "optional",
        &["amd", "nvidia"],
        &format!("{{comfy: {{amd: {AMD_IMAGE}, nvidia: {NVIDIA_IMAGE}}}}}"),
    );
    m.install(&source, &granted(), super::langs())
        .await
        .unwrap();
    assert_eq!(image_of(&docker, "comfy"), json!(AMD_IMAGE));
    assert_eq!(image_of(&docker, "web"), json!("nginx"));
    assert!(
        env_of(&docker, "comfy").contains(&"VENDOR=amd".to_owned()),
        "{:?}",
        env_of(&docker, "comfy")
    );
}

#[tokio::test]
async fn the_vendor_comes_first_in_the_list_even_when_the_host_has_both() {
    // 本机有 NVIDIA 驱动和运行时：作者把 nvidia 写在前面，就用 NVIDIA。
    let host = FakeHost::nvidia();
    let (docker, m, _dir) = on(&host, &["runc", "nvidia"]).await;
    let source = with_images(
        "optional",
        &["nvidia", "amd"],
        &format!("{{comfy: {{amd: {AMD_IMAGE}, nvidia: {NVIDIA_IMAGE}}}}}"),
    );
    m.install(&source, &granted(), super::langs())
        .await
        .unwrap();
    assert_eq!(image_of(&docker, "comfy"), json!(NVIDIA_IMAGE));
    let host_config = docker
        .created(&container_name("app-test", "comfy"))
        .unwrap()["HostConfig"]
        .clone();
    assert_eq!(host_config["DeviceRequests"][0]["Count"], json!(-1));
    assert!(
        host_config["Devices"].is_null(),
        "选中 NVIDIA 时不给 DRM 设备"
    );
}

#[tokio::test]
async fn a_vendor_whose_image_lacks_the_host_arch_is_skipped() {
    let host = FakeHost::amd();
    let (docker, m, _dir) = on(&host, &["runc"]).await;
    docker.set_arch("aarch64");
    docker.set_platforms(AMD_IMAGE, &["amd64"], None);
    docker.set_platforms(NVIDIA_IMAGE, &["arm64"], None);
    // 本机只有 AMD 的卡，但写的两家都支持；先写的那家镜像只有 amd64，跳过它没有下一家能选，原因写清。
    let source = with_images(
        "required",
        &["amd", "nvidia"],
        &format!("{{comfy: {{amd: {AMD_IMAGE}, nvidia: {NVIDIA_IMAGE}}}}}"),
    );
    let error = m
        .install(&source, &granted(), super::langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "gpu_unavailable");
    assert!(error.to_string().contains("aarch64"), "{error}");
}

#[tokio::test]
async fn an_optional_plugin_without_a_usable_vendor_falls_back_to_the_generic_image() {
    let host = FakeHost::amd();
    let (docker, m, _dir) = on(&host, &["runc", "nvidia"]).await;
    // 插件只支持 NVIDIA，本机只有 AMD。
    let source = with_images(
        "optional",
        &["nvidia"],
        &format!("{{comfy: {{nvidia: {NVIDIA_IMAGE}}}}}"),
    );
    let preview = preview(&m, &source).await;
    assert_eq!(preview.blocked, None);
    let notice = preview
        .notices
        .iter()
        .find(|line| line.text.contains("显卡"))
        .expect("预览里写明本机没有支持的显卡");
    assert!(notice.text.contains("NVIDIA"), "{}", notice.text);
    m.install(&source, &granted(), super::langs())
        .await
        .unwrap();
    assert_eq!(image_of(&docker, "comfy"), json!(GENERIC));
    assert!(
        env_of(&docker, "comfy").contains(&"VENDOR=none".to_owned()),
        "{:?}",
        env_of(&docker, "comfy")
    );
}

#[tokio::test]
async fn a_required_plugin_without_a_usable_vendor_does_not_install() {
    let host = FakeHost::amd();
    let (_docker, m, _dir) = on(&host, &["runc", "nvidia"]).await;
    let source = with_images(
        "required",
        &["nvidia"],
        &format!("{{comfy: {{nvidia: {NVIDIA_IMAGE}}}}}"),
    );
    let preview = preview(&m, &source).await;
    let blocked = preview.blocked.expect("装不了，原因在 blocked");
    assert!(blocked.contains("AMD"), "{blocked}");
    let error = m
        .install(&source, &granted(), super::langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "gpu_unavailable");
}

#[tokio::test]
async fn the_preview_names_the_vendor_image_and_its_size() {
    let host = FakeHost::amd();
    let (docker, m, _dir) = on(&host, &["runc"]).await;
    docker.set_platforms(AMD_IMAGE, &["amd64"], Some(23 * 1024 * 1024 * 1024));
    let source = with_images(
        "optional",
        &["amd"],
        &format!("{{comfy: {{amd: {AMD_IMAGE}}}}}"),
    );
    let notice = preview(&m, &source)
        .await
        .notices
        .into_iter()
        .find(|line| line.text.contains(AMD_IMAGE))
        .expect("预览里写了厂商和镜像");
    assert!(notice.text.contains(AMD_IMAGE), "{}", notice.text);
    assert!(notice.text.contains("23 GB"), "{}", notice.text);
}

#[tokio::test]
async fn an_unreadable_platform_list_is_shown_but_does_not_block() {
    let host = FakeHost::amd();
    let (docker, m, _dir) = on(&host, &["runc"]).await;
    docker.set_platform_error("offline");
    let source = with_images(
        "required",
        &["amd"],
        &format!("{{comfy: {{amd: {AMD_IMAGE}}}}}"),
    );
    let preview = preview(&m, &source).await;
    assert_eq!(preview.blocked, None);
    let notice = preview
        .notices
        .iter()
        .find(|line| line.text.contains("查不出"))
        .expect("预览里如实写查不出来");
    assert!(notice.text.contains(AMD_IMAGE), "{}", notice.text);
    m.install(&source, &granted(), super::langs())
        .await
        .unwrap();
    assert_eq!(image_of(&docker, "comfy"), json!(AMD_IMAGE));
}

#[tokio::test]
async fn the_recorded_vendor_is_what_a_rebuild_uses() {
    let host = FakeHost::amd();
    let (docker, m, dir) = on(&host, &["runc"]).await;
    let source = with_images(
        "optional",
        &["amd", "nvidia"],
        &format!("{{comfy: {{amd: {AMD_IMAGE}, nvidia: {NVIDIA_IMAGE}}}}}"),
    );
    m.install(&source, &granted(), super::langs())
        .await
        .unwrap();
    let record = m
        .installed_record(&super::super::manifest::PluginId::parse("app.test").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(record.gpu_vendor, Some(GpuVendor::Amd));
    // 本机换了显卡（现在只有 NVIDIA 驱动）：重建仍用记录里的 AMD 版镜像，不悄悄换。
    let nvidia_host = FakeHost::nvidia();
    let m = std::sync::Arc::new(m.with_host(nvidia_host.paths()));
    m.start_detached(super::super::manifest::PluginId::parse("app.test").unwrap())
        .await
        .unwrap();
    let _ = &dir;
    assert_eq!(image_of(&docker, "comfy"), json!(AMD_IMAGE));
}

#[tokio::test]
async fn a_plugin_without_vendor_images_keeps_the_generic_image_and_gets_the_variable() {
    let host = FakeHost::amd();
    let (docker, m, _dir) = on(&host, &["runc"]).await;
    let source = compose(
        "  gpu: {services: [comfy]}\n  permissions: [devices.gpu]\n  \
         requirements: {gpu: {need: optional, vendors: [amd]}}\n",
    );
    m.install(&source, &granted(), super::langs())
        .await
        .unwrap();
    assert_eq!(image_of(&docker, "comfy"), json!(GENERIC));
    assert!(env_of(&docker, "comfy").contains(&"VENDOR=amd".to_owned()));
}

#[tokio::test]
async fn the_core_in_a_container_without_the_host_dev_mount_does_not_guess() {
    let host = FakeHost::amd();
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_in_container(engine, &dir).with_host(host.paths_without_dev());
    let source = with_images(
        "optional",
        &["amd"],
        &format!("{{comfy: {{amd: {AMD_IMAGE}}}}}"),
    );
    m.install(&source, &granted(), super::langs())
        .await
        .unwrap();
    // 看得到 sysfs、看不到宿主的 `/dev`：optional 的照常装，退回通用镜像、不给设备。
    assert_eq!(image_of(&docker, "comfy"), json!(GENERIC));
    assert!(
        docker
            .created(&container_name("app-test", "comfy"))
            .unwrap()["HostConfig"]["Devices"]
            .is_null()
    );
    assert!(env_of(&docker, "comfy").contains(&"VENDOR=none".to_owned()));
}

#[tokio::test]
async fn switching_the_optional_permission_selects_and_drops_the_vendor() {
    let host = FakeHost::amd();
    let (docker, engine) = FakeDocker::start().await;
    docker.set_runtimes(&["runc"]);
    let dir = tempfile::tempdir().unwrap();
    let m = std::sync::Arc::new(manager_with(engine, &dir).with_host(host.paths()));
    let source = package(
        &format!(
            "  comfy:\n    image: {GENERIC}\n    environment: {{VENDOR: '${{WONTON_GPU_VENDOR:-none}}'}}\n"
        ),
        &format!(
            "  gpu: {{services: [comfy], images: {{comfy: {{amd: {AMD_IMAGE}}}}}}}\n  \
             optional_permissions:\n    devices.gpu:\n      off_loss: {{zh: 没有显卡, en: No GPU}}\n  \
             requirements: {{gpu: {{need: optional, vendors: [amd]}}}}\n  \
             permissions: []\n"
        ),
    );
    let package = PluginPackage::parse(&source, &super::langs()).unwrap();
    m.install_user_enabled(package, &[], &[], false, Default::default(), super::langs())
        .await
        .unwrap();
    assert_eq!(image_of(&docker, "comfy"), json!(GENERIC));

    let id = PluginId::parse("app.test").unwrap();
    m.set_plugin_permission(
        id.clone(),
        Permission::DevicesGpu,
        true,
        OpenedBy::Owner,
        super::langs(),
    )
    .await
    .unwrap();
    assert_eq!(image_of(&docker, "comfy"), json!(AMD_IMAGE));
    assert!(env_of(&docker, "comfy").contains(&"VENDOR=amd".to_owned()));
    let record = m.installed_record(&id).unwrap().unwrap();
    assert_eq!(record.gpu_vendor, Some(GpuVendor::Amd));

    m.set_plugin_permission(
        id.clone(),
        Permission::DevicesGpu,
        false,
        OpenedBy::Owner,
        super::langs(),
    )
    .await
    .unwrap();
    assert_eq!(image_of(&docker, "comfy"), json!(GENERIC));
    assert!(env_of(&docker, "comfy").contains(&"VENDOR=none".to_owned()));
    let record = m.installed_record(&id).unwrap().unwrap();
    assert_eq!(record.gpu_vendor, None);
}
