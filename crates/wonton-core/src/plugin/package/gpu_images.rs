//! 清单 `gpu.images` 对照 compose 的检查（ADR-0099）：厂商版镜像只换 `image` 一个字段，所以服务要在 compose 里，
//! 厂商版镜像也不能和通用镜像一样——一样多半是拼错了，等于没写。

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
        let Some(compose) = services.get(service) else {
            problems.push(text(
                langs,
                "plugin-install-pkg-gpu-images-service",
                &[("service", service.as_str().into())],
            ));
            continue;
        };
        let generic = compose.get("image").and_then(Value::as_str);
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
