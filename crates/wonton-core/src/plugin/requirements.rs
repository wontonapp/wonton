//! 插件对宿主的要求（清单的 `requirements`，见插件规范）：都是机器能判断的结构化字段。

use std::collections::BTreeSet;

use super::languages::Languages;
use super::manifest::{Manifest, ManifestError};
use crate::l10n::{list, text};

pub use wonton_protocol::manifest::{Arch, GpuNeed, GpuRequirement, GpuVendor, Requirements};

/// `512M`、`2G`、`1.5G` 换算成字节（1G = 1024M）。别的写法不算。
pub fn parse_size(text: &str) -> Option<u64> {
    let (number, unit) = text.split_at(text.len().checked_sub(1)?);
    let unit: f64 = match unit {
        "M" => 1024.0 * 1024.0,
        "G" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    if !number.starts_with(|c: char| c.is_ascii_digit())
        || !number.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        return None;
    }
    let number: f64 = number.parse().ok()?;
    let bytes = number * unit;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (bytes.is_finite() && bytes >= 1.0).then_some(bytes as u64)
}

pub fn validate(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    let Some(requirements) = &manifest.requirements else {
        return Ok(());
    };
    let gpu = requirements.gpu.as_ref();
    let sizes = [
        ("requirements.memory", requirements.memory.as_deref()),
        (
            "requirements.gpu.memory",
            gpu.and_then(|g| g.memory.as_deref()),
        ),
    ];
    for (field, size) in sizes {
        if let Some(size) = size
            && parse_size(size).is_none()
        {
            return Err(ManifestError::Invalid(text(
                langs,
                "plugin-install-req-size-format",
                &[("field", field.into()), ("size", size.into())],
            )));
        }
    }
    if let Some(vendors) = gpu.map(|gpu| gpu.vendors.as_slice()) {
        let mut seen = BTreeSet::new();
        let mut problems = Vec::new();
        for vendor in vendors {
            if *vendor == GpuVendor::Other {
                problems.push(text(langs, "plugin-install-req-vendor-unknown", &[]));
            } else if !seen.insert(*vendor) {
                problems.push(text(
                    langs,
                    "plugin-install-req-vendor-duplicated",
                    &[("vendor", vendor.key().into())],
                ));
            }
        }
        if !problems.is_empty() {
            return Err(ManifestError::Invalid(problems.join("; ")));
        }
    }
    Ok(())
}

pub fn arch_line(arch: &[Arch], langs: &Languages) -> String {
    let names: Vec<String> = arch.iter().map(|a| a.name().to_owned()).collect();
    text(
        langs,
        "plugin-install-req-arch-line",
        &[("arches", list(langs, &names).into())],
    )
}

pub fn memory_line(memory: &str, langs: &Languages) -> String {
    text(
        langs,
        "plugin-install-req-memory-line",
        &[("memory", memory.into())],
    )
}

pub fn gpu_line(gpu: &GpuRequirement, langs: &Languages) -> String {
    let card = card_name(&gpu.vendors, langs);
    let memory = gpu.memory.as_ref().map(|memory| {
        text(
            langs,
            "plugin-install-req-gpu-vram-suffix",
            &[("memory", memory.as_str().into())],
        )
    });
    match gpu.need {
        GpuNeed::Required => text(
            langs,
            "plugin-install-req-gpu-required",
            &[
                ("card", card.as_str().into()),
                ("memory", memory.unwrap_or_default().into()),
            ],
        ),
        GpuNeed::Optional => text(
            langs,
            "plugin-install-req-gpu-optional",
            &[
                ("card", card.as_str().into()),
                ("memory", memory.unwrap_or_default().into()),
            ],
        ),
    }
}

/// `显卡`，或者 ` NVIDIA 或 AMD 显卡`：直接接在中文后面，开头的英文前留空格。
pub fn card_name(vendors: &[GpuVendor], langs: &Languages) -> String {
    match vendors {
        [] => text(langs, "plugin-install-req-card-generic", &[]),
        vendors => {
            let names: Vec<&str> = vendors.iter().map(|v| v.name()).collect();
            let or = text(langs, "plugin-install-or-separator", &[]);
            text(
                langs,
                "plugin-install-req-card-named",
                &[("names", format!(" {}", names.join(&or)).into())],
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_size;

    #[test]
    fn sizes_are_a_number_and_a_unit() {
        assert_eq!(parse_size("512M"), Some(512 * 1024 * 1024));
        assert_eq!(parse_size("2G"), Some(2 * 1024 * 1024 * 1024));
        assert_eq!(parse_size("1.5G"), Some(1536 * 1024 * 1024));
        for bad in ["", "G", "2", "2GB", "2g", "-1G", "0G", "1e9M", "infG"] {
            assert_eq!(parse_size(bad), None, "{bad}");
        }
    }
}
