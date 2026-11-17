use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::PortSpec;

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

    pub fn name(self) -> &'static str {
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
    /// 显存下限，写法为 `512M`、`2G` 或 `1.5G`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirements {
    #[serde(default)]
    pub arch: Option<Vec<Arch>>,
    /// 宿主内存下限，写法为 `512M`、`2G` 或 `1.5G`。
    #[serde(default)]
    pub memory: Option<String>,
    #[serde(default)]
    pub gpu: Option<GpuRequirement>,
}

/// 服务 → 厂商 → 完整的镜像引用。
pub type GpuImages = BTreeMap<String, BTreeMap<GpuVendor, String>>;

/// 清单的 `gpu`：拿到显卡的服务。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuSpec {
    pub services: Vec<String>,
    /// 按厂商换的镜像（ADR-0099），键是 `services` 里的服务。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub images: GpuImages,
}

/// 清单的 `lan`：在局域网上开放的端口。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanSpec {
    pub ports: Vec<PortSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<LanClientSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanClientSpec {
    /// 连接地址，`{host}` 换成端口开着的局域网地址，如 `http://{host}:4533`。
    pub address: String,
    /// 主人的账号模板；不能用显示名 `{name}` 或网页请求者的 `{role}`。
    pub username: String,
    pub password: String,
}
