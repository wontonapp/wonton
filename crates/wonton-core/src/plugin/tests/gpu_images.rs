//! 清单 `gpu.images` 和 `requirements.gpu.vendors`（ADR-0099）：按厂商换镜像的写法、引用和先后顺序。

use super::super::package::{PackageError, PluginPackage};
use super::super::requirements::GpuVendor;
use super::langs;

const SERVICE: &str = "  comfy:\n    image: example/comfy@sha256:1111111111111111111111111111111111111111111111111111111111111111\n";

fn compose(services: &str, manifest: &str) -> String {
    format!(
        "services:\n{services}x-wonton:\n  spec: '0.14'\n  id: app.test\n  version: 1.0.0\n  kind: app\n  \
         title: {{zh: 测试, en: Test}}\n  description: {{zh: 测试, en: Test}}\n{manifest}"
    )
}

/// 一个服务，装了显卡、给了权限：`extra` 接在 `gpu.services` 后面。
fn gpu(extra: &str) -> String {
    format!(
        "  gpu: {{services: [comfy]{extra}}}\n  permissions: [devices.gpu]\n  \
         requirements:\n    gpu: {{need: required, vendors: [amd, nvidia]}}\n"
    )
}

fn parse_error(source: &str) -> String {
    PluginPackage::parse(source, &langs())
        .expect_err("清单应不合法")
        .to_string()
}

#[test]
fn vendor_images_parse_and_round_trip_in_priority_order() {
    let source = compose(
        SERVICE,
        &gpu(", images: {comfy: {amd: example/comfy:rocm, nvidia: example/comfy:cuda}}"),
    );
    let package = PluginPackage::parse(&source, &langs()).unwrap();
    let images = &package.manifest.gpu.as_ref().unwrap().images;
    assert_eq!(images["comfy"][&GpuVendor::Amd], "example/comfy:rocm");
    assert_eq!(images["comfy"][&GpuVendor::Nvidia], "example/comfy:cuda");
    let vendors = &package
        .manifest
        .requirements
        .as_ref()
        .unwrap()
        .gpu
        .as_ref()
        .unwrap()
        .vendors;
    assert_eq!(vendors, &[GpuVendor::Amd, GpuVendor::Nvidia]);
}

#[test]
fn images_without_vendors_are_rejected() {
    let source = compose(
        SERVICE,
        "  gpu: {services: [comfy], images: {comfy: {amd: example/comfy:rocm}}}\n  \
         permissions: [devices.gpu]\n",
    );
    let error = parse_error(&source);
    assert!(error.contains("requirements.gpu.vendors"), "{error}");
}

#[test]
fn an_image_service_must_be_in_gpu_services() {
    let source = compose(
        SERVICE,
        "  gpu: {services: [web], images: {comfy: {amd: example/comfy:rocm}}}\n  \
         permissions: [devices.gpu]\n  requirements:\n    gpu: {vendors: [amd]}\n",
    );
    // `web` 不在 compose 里也一并报出来。
    let error = parse_error(&source);
    assert!(error.contains("gpu.services"), "{error}");
}

#[test]
fn an_image_vendor_must_be_in_the_vendors_list() {
    let source = compose(
        SERVICE,
        &gpu(", images: {comfy: {intel: example/comfy:sycl}}"),
    );
    let error = parse_error(&source);
    assert!(error.contains("requirements.gpu.vendors"), "{error}");
}

#[test]
fn every_manifest_problem_is_reported_in_one_go() {
    let source = compose(
        SERVICE,
        "  gpu: {services: [comfy], images: {comfy: {intel: '', amd: 'example/comfy:rocm rocm'}, \
         web: {amd: example/web:rocm}}}\n  permissions: [devices.gpu]\n  \
         requirements:\n    gpu: {vendors: [amd]}\n",
    );
    let error = parse_error(&source);
    for problem in [
        "gpu.images.comfy.intel is empty",
        "gpu.images.comfy.amd has whitespace",
        "gpu.images has service web",
    ] {
        assert!(error.contains(problem), "{problem}: {error}");
    }
}

#[test]
fn every_compose_problem_is_reported_in_one_go() {
    let source = compose(
        &format!(
            "{SERVICE}  web:\n    image: example/web@sha256:2222222222222222222222222222222222222222222222222222222222222222\n"
        ),
        "  gpu: {services: [comfy, web], images: {comfy: {amd: example/comfy@sha256:1111111111111111111111111111111111111111111111111111111111111111}, \
         web: {amd: example/web@sha256:2222222222222222222222222222222222222222222222222222222222222222}}}\n  \
         permissions: [devices.gpu]\n  requirements:\n    gpu: {vendors: [amd]}\n",
    );
    let error = parse_error(&source);
    assert!(error.contains("gpu.images.comfy.amd"), "{error}");
    assert!(error.contains("gpu.images.web.amd"), "{error}");
}

#[test]
fn an_unknown_vendor_parses_as_other_and_asks_for_an_update() {
    let source = compose(
        SERVICE,
        "  gpu: {services: [comfy], images: {comfy: {apple: example/comfy:metal}}}\n  \
         permissions: [devices.gpu]\n  requirements:\n    gpu: {vendors: [apple]}\n",
    );
    let error = parse_error(&source);
    assert!(error.contains("amd, nvidia, intel"), "{error}");
}

#[test]
fn vendors_cannot_repeat() {
    let source = compose(
        SERVICE,
        &gpu("").replace("[amd, nvidia]", "[amd, nvidia, amd]"),
    );
    let error = parse_error(&source);
    assert!(error.contains("amd"), "{error}");
}

/// 厂商版镜像只换 `image`：服务得在 `gpu.services` 里，那它也必须在 compose 里（由 `gpu.services` 那一步报）。
#[test]
fn a_vendor_image_service_must_be_in_compose() {
    let source = compose(
        SERVICE,
        "  gpu: {services: [comfy, web], images: {web: {amd: example/web:rocm}}}\n  \
         permissions: [devices.gpu]\n  requirements:\n    gpu: {need: required, vendors: [amd]}\n",
    );
    let error = parse_error(&source);
    assert!(error.contains("gpu.services"), "{error}");
    assert!(error.contains("`web`"), "{error}");
}

#[test]
fn a_vendor_image_equal_to_the_generic_one_is_rejected() {
    let source = compose(
        SERVICE,
        &gpu(
            ", images: {comfy: {amd: example/comfy@sha256:1111111111111111111111111111111111111111111111111111111111111111}}",
        ),
    );
    let error = parse_error(&source);
    assert!(error.contains("same as the service's image"), "{error}");
}

/// 字段进了 0.14 的规范：`spec` 写得低时 `plugin check` 让作者抬版本，而不是让旧核心报未知字段。
#[test]
fn images_need_spec_0_14() {
    let mut document = serde_json::json!({
        "services": {"comfy": {"image": "example/comfy"}},
        "x-wonton": {
            "spec": "0.13",
            "gpu": {"services": ["comfy"], "images": {"comfy": {"amd": "example/comfy:rocm"}}},
        },
    });
    let found = super::super::spec_floor::problems(&document);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("x-wonton.gpu.images"), "{found:?}");
    assert!(found[0].contains("\"0.14\""), "{found:?}");
    document["x-wonton"]["spec"] = serde_json::json!("0.14");
    assert!(super::super::spec_floor::problems(&document).is_empty());
}

/// 注入变量的引用也算：写了它就要 `spec: "0.14"`，旧核心才好先让人升级而不是报“没有注入这个变量”。
#[test]
fn referencing_the_vendor_variable_needs_spec_0_14_too() {
    let mut document = serde_json::json!({
        "services": {"comfy": {"image": "example/comfy", "environment": {"VENDOR": "${WONTON_GPU_VENDOR}"}}},
        "x-wonton": {"spec": "0.13"},
    });
    let found = super::super::spec_floor::problems(&document);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("WONTON_GPU_VENDOR"), "{found:?}");
    document["x-wonton"]["spec"] = serde_json::json!("0.14");
    assert!(super::super::spec_floor::problems(&document).is_empty());
}

#[test]
fn compose_errors_are_the_gpu_images_category() {
    let source = compose(
        SERVICE,
        "  gpu: {services: [comfy], images: {comfy: {amd: example/comfy@sha256:1111111111111111111111111111111111111111111111111111111111111111}}}\n  \
         permissions: [devices.gpu]\n  requirements:\n    gpu: {vendors: [amd]}\n",
    );
    let error = PluginPackage::parse(&source, &langs()).unwrap_err();
    assert!(matches!(error, PackageError::GpuImages(_)), "{error}");
}
