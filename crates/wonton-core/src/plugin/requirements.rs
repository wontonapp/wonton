//! 插件对宿主的要求（清单的 `requirements`，见插件规范）：都是机器能判断的结构化字段。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::languages::Languages;
use super::manifest::{Manifest, ManifestError};
use crate::l10n::{list, text};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    Amd64,
    Arm64,
}

impl Arch {
    /// 把 Docker 报告的架构名（`x86_64`、`aarch64` 等）归一。
    pub fn from_docker(arch: &str) -> Option<Self> {
        match arch {
            "amd64" | "x86_64" => Some(Self::Amd64),
            "arm64" | "aarch64" => Some(Self::Arm64),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuNeed {
    /// 没有符合的显卡就装不了。
    #[default]
    Required,
    /// 照常装；没有符合的显卡时不给设备，插件自己退回 CPU。
    Optional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuVendor {
    Amd,
    Nvidia,
    Intel,
    /// 比这个核心新的厂商：解析得出来，校验时提示先升级核心（探不到它，也就选不到）。
    #[serde(other)]
    Other,
}

impl GpuVendor {
    /// 清单里的写法。
    pub fn key(self) -> &'static str {
        match self {
            Self::Amd => "amd",
            Self::Nvidia => "nvidia",
            Self::Intel => "intel",
            Self::Other => "other",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Amd => "AMD",
            Self::Nvidia => "NVIDIA",
            Self::Intel => "Intel",
            Self::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuRequirement {
    #[serde(default)]
    pub need: GpuNeed,
    /// 能用的厂商；空表示哪家都行。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vendors: Vec<GpuVendor>,
    /// 显存下限，写法同 [`parse_size`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirements {
    #[serde(default)]
    pub arch: Option<Vec<Arch>>,
    /// 宿主内存下限，写法同 [`parse_size`]。
    #[serde(default)]
    pub memory: Option<String>,
    #[serde(default)]
    pub gpu: Option<GpuRequirement>,
}

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
