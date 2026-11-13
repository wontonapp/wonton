//! 清单 `x-wonton.gpu.images`：按宿主显卡的厂商换服务的镜像（ADR-0099）。compose 里的 `image` 是通用镜像，
//! 没有专门版本的厂商、没给显卡时都用它；这张表只换 `image` 一个字段，可以换仓库（vLLM 的 AMD 版就是另一个仓库）。
//!
//! 这里对照清单自己的 `gpu.services`、`requirements.gpu.vendors` 和镜像的写法；服务在 compose 里存不存在、
//! 厂商版镜像和通用镜像是不是一样由 [`super::package`] 检查。

use std::collections::{BTreeMap, BTreeSet};

use super::languages::Languages;
use super::manifest::{Manifest, ManifestError};
use super::requirements::GpuVendor;
use crate::l10n::text;

/// 服务 → 厂商 → 完整的镜像引用。
pub type GpuImages = BTreeMap<String, BTreeMap<GpuVendor, String>>;

/// 一处问题一条，一次报全。
pub(super) fn validate(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    let Some(gpu) = &manifest.gpu else {
        return Ok(());
    };
    let images = &gpu.images;
    if images.is_empty() {
        return Ok(());
    }
    let vendors = manifest
        .requirements
        .as_ref()
        .and_then(|requirements| requirements.gpu.as_ref())
        .map_or(&[][..], |gpu| gpu.vendors.as_slice());
    let mut problems = Vec::new();
    // 不写 `vendors` 等于哪家都行，Intel 主机上就会拿通用镜像去配 Intel 的设备，而作者只想过别的厂商。
    if vendors.is_empty() {
        problems.push(text(langs, "plugin-install-gpu-images-need-vendors", &[]));
    }
    let services: BTreeSet<&str> = gpu.services.iter().map(String::as_str).collect();
    for (service, by_vendor) in images {
        if !services.contains(service.as_str()) {
            problems.push(text(
                langs,
                "plugin-install-gpu-images-service-unknown",
                &[("service", service.as_str().into())],
            ));
        }
        for (vendor, image) in by_vendor {
            if !vendors.contains(vendor) {
                problems.push(text(
                    langs,
                    "plugin-install-gpu-images-vendor-unknown",
                    &[
                        ("service", service.as_str().into()),
                        ("vendor", vendor.key().into()),
                    ],
                ));
            }
            let field = format!("gpu.images.{service}.{}", vendor.key());
            // 镜像的仓库名、标签合不合法由 Docker 判断，这里只拦下明显不是一个引用的写法。
            if image.is_empty() {
                problems.push(text(
                    langs,
                    "plugin-install-gpu-images-empty",
                    &[("field", field.as_str().into())],
                ));
            } else if image.chars().any(char::is_whitespace) {
                problems.push(text(
                    langs,
                    "plugin-install-gpu-images-whitespace",
                    &[("field", field.as_str().into())],
                ));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(ManifestError::Invalid(problems.join("; ")))
    }
}
