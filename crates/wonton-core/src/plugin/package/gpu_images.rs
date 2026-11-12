//! 清单 `gpu.images` 对照 compose 的检查（ADR-0099）：厂商版镜像不能和服务自己的`image`一样——一样多半是拼错了，
//! 等于没写。服务在 compose 里存不存在由 [`super::PluginPackage::check_manifest_references`] 的 `gpu.services` 那一步报
//! （`images` 的服务必须在 `gpu.services` 里，清单校验已经保证）。

use serde_json::{Map, Value};

use super::super::languages::Languages;
use super::super::manifest::GpuImages;
use super::PackageError;
use crate::l10n::text;

/// 一处问题一条，一次报全。
pub(super) fn check(
    images: &GpuImages,
    services: &Map<String, Value>,
    langs: &Languages,
) -> Result<(), PackageError> {
    let mut problems = Vec::new();
    for (service, by_vendor) in images {
        // 走到这里时服务一定在 compose 里（`gpu.services` 先查过）。
        let generic = services
            .get(service)
            .and_then(|compose| compose.get("image"))
            .and_then(Value::as_str);
        for (vendor, image) in by_vendor {
            if generic == Some(image.as_str()) {
                problems.push(text(
                    langs,
                    "plugin-install-pkg-gpu-image-same",
                    &[
                        ("service", service.as_str().into()),
                        ("vendor", vendor.key().into()),
                    ],
                ));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(PackageError::GpuImages(problems.join("; ")))
    }
}
