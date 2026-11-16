//! 把宿主的显卡交给插件（ADR-0018）。清单的 `gpu.services` 说明哪些服务用显卡，`devices.gpu` 权限由主人确认；
//! 交给容器什么由核心按宿主实际有的显卡决定，不收插件 compose 里自己写的 `devices`。
//!
//! 探测只看文件：sysfs 是内核的，核心在容器里看到的和宿主上一样，由它判断有没有显卡、加载了什么驱动；设备节点
//! 和它们所属的组在宿主的 `/dev` 里，核心在容器里时要由 compose 只读挂载进来（`WONTON_HOST_DEV`）。NVIDIA 的
//! 设备不由核心列举，交给 Docker 的 `DeviceRequests`（NVIDIA Container Toolkit），核心只确认驱动和运行时都在。

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use bollard::models::{DeviceMapping, DeviceRequest, HostConfig};
use wonton_protocol::CardLine;

use super::host_check;
use super::languages::Languages;
use super::manifest::{GpuNeed, Manifest, ManifestError, Permission};
use super::requirements::GpuVendor;
use crate::l10n::{list, text};

pub use wonton_protocol::manifest::GpuSpec;

/// `gpu`、`devices.gpu` 和 `requirements.gpu` 三者要一起出现：只写其中一个说明清单写错了，而不是不要显卡。
/// 服务名是否存在由 [`super::package`] 对照 compose 检查。
pub fn validate(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    let invalid = |message: String| Err(ManifestError::Invalid(message));
    let optional = manifest
        .optional_permissions
        .contains_key(&Permission::DevicesGpu);
    let permitted = manifest.permissions.contains(&Permission::DevicesGpu) || optional;
    let wants = manifest.requirements.as_ref().and_then(|r| r.gpu.as_ref());
    match (&manifest.gpu, permitted) {
        (Some(_), false) => {
            return invalid(text(langs, "plugin-install-gpu-needs-permission", &[]));
        }
        (None, true) => {
            return invalid(text(langs, "plugin-install-gpu-needs-services", &[]));
        }
        (None, false) if wants.is_some() => {
            return invalid(text(langs, "plugin-install-gpu-needs-all-three", &[]));
        }
        _ => {}
    }
    if optional && wants.is_some_and(|gpu| gpu.need == GpuNeed::Required) {
        return invalid(text(
            langs,
            "plugin-install-optional-gpu-required",
            &[("permission", "devices.gpu".into())],
        ));
    }
    if let Some(gpu) = &manifest.gpu {
        let mut seen = BTreeSet::new();
        if gpu.services.is_empty() || !gpu.services.iter().all(|s| seen.insert(s)) {
            return invalid(text(langs, "plugin-install-gpu-services-nonempty", &[]));
        }
    }
    Ok(())
}

/// 插件要不要显卡：没申请 `devices.gpu` 的不要；可选的按 `optional`；必需的而没写 `requirements.gpu` 的按 `required`。
pub fn requirement(manifest: &Manifest) -> Option<GpuNeed> {
    if manifest
        .optional_permissions
        .contains_key(&Permission::DevicesGpu)
    {
        return Some(GpuNeed::Optional);
    }
    manifest
        .permissions
        .contains(&Permission::DevicesGpu)
        .then(|| {
            manifest
                .requirements
                .as_ref()
                .and_then(|r| r.gpu.as_ref())
                .map_or(GpuNeed::Required, |gpu| gpu.need)
        })
}

/// 核心看到的宿主路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPaths {
    /// 宿主的 `/dev` 在核心里的路径。核心在容器里却没挂载它时为 `None`：探测如实报错，不把容器自己的 `/dev`
    /// 当成宿主的。
    pub dev: Option<PathBuf>,
    /// 宿主的 `/run/udev` 在核心里的路径（`WONTON_HOST_UDEV`）：宿主没有 udev 时为空，设备自动发现少一份数据库。
    pub udev: Option<PathBuf>,
    /// sysfs。测试里换成假的目录树。
    pub sys: PathBuf,
    /// procfs，同上。
    pub proc: PathBuf,
}

impl HostPaths {
    /// 核心直接跑在宿主上时就是 `/dev`；在容器里时要配置挂载进来的路径（`WONTON_HOST_DEV`）。
    pub fn new(host_dev: Option<PathBuf>, host_udev: Option<PathBuf>) -> Self {
        Self {
            dev: host_dev,
            udev: host_udev,
            sys: PathBuf::from("/sys"),
            proc: PathBuf::from("/proc"),
        }
    }
}

/// 探测到的、本机能交出去的全部显卡：还没选厂商。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbedGpu {
    /// 有可用驱动的厂商。
    pub vendors: BTreeSet<GpuVendor>,
    /// 宿主上的设备路径，原样出现在容器里：`/dev/dri`（整个目录，含渲染节点），有 KFD 时还有 `/dev/kfd`。
    pub devices: Vec<String>,
    /// 这些设备节点所属的组的 GID（通常是宿主的 `video`、`render`），容器加入它们，非 root 的进程才开得了设备。
    pub groups: Vec<u32>,
    /// 全部 NVIDIA 显卡：经 Docker 的 `DeviceRequests` 由 NVIDIA Container Toolkit 放进容器，同 `--gpus all`。
    pub nvidia: bool,
}

/// 交给服务的显卡：选中那家的设备，和服务按这家换上的镜像。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuGrant {
    /// 选中的厂商；本机现在没有这家的卡时仍然保留（镜像照记录换，设备由容器自己报错）。
    pub vendor: GpuVendor,
    /// 宿主上的设备路径，原样出现在容器里：`/dev/dri`（整个目录，含渲染节点），有 KFD 时还有 `/dev/kfd`。
    pub devices: Vec<String>,
    /// 这些设备节点所属的组的 GID（通常是宿主的 `video`、`render`），容器加入它们，非 root 的进程才开得了设备。
    pub groups: Vec<u32>,
    /// 全部 NVIDIA 显卡：经 Docker 的 `DeviceRequests` 由 NVIDIA Container Toolkit 放进容器，同 `--gpus all`。
    pub nvidia: bool,
    /// 选中那家厂商的厂商版镜像（见 [`GpuImage`]）。
    pub images: Vec<GpuImage>,
    /// 读不到平台列表的镜像：预览里如实写查不出来，不拦安装。
    pub arch_unknown: Vec<String>,
}

/// 按厂商换上的镜像：`gpu.images.<服务>.<厂商>` 的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuImage {
    pub service: String,
    pub image: String,
    /// 压缩后的大小，预览里写出来；读不到时为空。
    pub size: Option<u64>,
}

impl ProbedGpu {
    /// 只留选中那家的设备。`/dev/dri` 是整个目录交出去的（理由见 ADR-0018），AMD 和 Intel 都用它；选中 NVIDIA 时
    /// 只走 `DeviceRequests`，别的厂商的 DRM 节点不给。
    pub fn keep(mut self, vendor: GpuVendor) -> GpuGrant {
        if vendor == GpuVendor::Nvidia {
            self.devices.clear();
            self.groups.clear();
            self.nvidia = true;
        }
        GpuGrant {
            vendor,
            devices: self.devices,
            groups: self.groups,
            nvidia: self.nvidia && vendor == GpuVendor::Nvidia,
            images: Vec::new(),
            arch_unknown: Vec::new(),
        }
    }
}

/// 本机没有记录里的那家显卡时：镜像照记录换，设备一个不给，容器自己报错（ADR-0018 的“装好后不再盯着显卡”）。
impl GpuImage {
    /// 预览里的一项：“服务 x 用 ROCm 版镜像 `y`（约 23 GB）”。
    fn line(&self, langs: &Languages) -> String {
        let size = self.size.map(|size| {
            text(
                langs,
                "plugin-install-gpu-image-size",
                &[("size", host_check::size(size).into())],
            )
        });
        text(
            langs,
            "plugin-install-gpu-image-line",
            &[
                ("service", self.service.as_str().into()),
                ("image", self.image.as_str().into()),
                ("size", size.unwrap_or_default().into()),
            ],
        )
    }
}

impl GpuGrant {
    pub fn images_only(vendor: GpuVendor, images: Vec<GpuImage>) -> Self {
        Self {
            vendor,
            devices: Vec::new(),
            groups: Vec::new(),
            nvidia: false,
            images,
            arch_unknown: Vec::new(),
        }
    }

    /// 给主人看的一句话：给了哪家的什么设备、用哪个镜像。
    pub fn describe(&self, langs: &Languages) -> String {
        let mut parts = Vec::new();
        if !self.devices.is_empty() {
            let groups: Vec<String> = self.groups.iter().map(ToString::to_string).collect();
            parts.push(text(
                langs,
                "plugin-install-gpu-describe-vendors",
                &[
                    ("vendors", self.vendor.name().into()),
                    ("devices", list(langs, &self.devices).into()),
                    ("groups", list(langs, &groups).into()),
                ],
            ));
        }
        if self.nvidia {
            parts.push(text(langs, "plugin-install-gpu-describe-nvidia", &[]));
        }
        parts.extend(self.images.iter().map(|image| image.line(langs)));
        parts.extend(self.arch_unknown.iter().cloned());
        parts.join("；")
    }

    /// 写进容器的 `HostConfig`。
    pub fn apply(&self, host: &mut HostConfig) {
        if !self.devices.is_empty() {
            host.devices = Some(
                self.devices
                    .iter()
                    .map(|path| DeviceMapping {
                        path_on_host: Some(path.clone()),
                        path_in_container: Some(path.clone()),
                        cgroup_permissions: Some("rwm".to_owned()),
                    })
                    .collect(),
            );
        }
        if !self.groups.is_empty() {
            host.group_add = Some(self.groups.iter().map(ToString::to_string).collect());
        }
        if self.nvidia {
            host.device_requests = Some(vec![DeviceRequest {
                count: Some(-1),
                capabilities: Some(vec![vec!["gpu".to_owned()]]),
                ..Default::default()
            }]);
        }
    }
}

/// 没有可用的显卡的原因，给主人看。变体只带数据；给 App 的文字用 [`GpuProblem::message`] 按语言拼。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuProblem {
    HostDevUnavailable,
    Unreadable {
        path: String,
        error: String,
    },
    NoDevice,
    NoDriver(String),
    NoRuntime,
    NoWantedVendor {
        host: String,
        wanted: String,
        reasons: Vec<String>,
    },
}

impl GpuProblem {
    pub fn message(&self, langs: &Languages) -> String {
        match self {
            Self::HostDevUnavailable => text(langs, "plugin-install-gpu-problem-host-dev", &[]),
            Self::Unreadable { path, error } => text(
                langs,
                "plugin-install-gpu-problem-unreadable",
                &[
                    ("path", path.as_str().into()),
                    ("error", error.as_str().into()),
                ],
            ),
            Self::NoDevice => text(langs, "plugin-install-gpu-problem-no-device", &[]),
            Self::NoDriver(detail) => text(
                langs,
                "plugin-install-gpu-problem-no-driver",
                &[("detail", detail.as_str().into())],
            ),
            Self::NoRuntime => text(langs, "plugin-install-gpu-problem-no-runtime", &[]),
            Self::NoWantedVendor {
                host,
                wanted,
                reasons,
            } if reasons.is_empty() => text(
                langs,
                "plugin-install-gpu-problem-no-wanted-vendor",
                &[
                    ("host", host.as_str().into()),
                    ("wanted", wanted.as_str().into()),
                ],
            ),
            Self::NoWantedVendor { reasons, .. } => text(
                langs,
                "plugin-install-gpu-problem-all-unusable",
                &[("reasons", list(langs, reasons).into())],
            ),
        }
    }
}

/// 按 sysfs 和宿主的 `/dev` 探测显卡。NVIDIA 只确认驱动已加载（`nvidia` 内核模块），运行时由调用方对照 Docker
/// 再查；`nvidia-smi` 之类的二进制装了不算，驱动没加载时它也在。
pub fn probe(paths: &HostPaths, langs: &Languages) -> Result<ProbedGpu, GpuProblem> {
    let sys = &paths.sys;
    let nvidia = sys.join("module/nvidia").is_dir();
    let render_vendors: Vec<String> = render_nodes(sys)?
        .iter()
        .map(|node| read_trimmed(&sys.join("class/drm").join(node).join("device/vendor")))
        .filter(|vendor| vendor != NVIDIA_VENDOR)
        .collect();
    if render_vendors.is_empty() && !nvidia {
        return Err(no_usable_driver(sys, langs)?);
    }
    let mut probed = ProbedGpu {
        vendors: BTreeSet::new(),
        devices: Vec::new(),
        groups: Vec::new(),
        nvidia,
    };
    if !render_vendors.is_empty() {
        let dev = paths.dev.as_ref().ok_or(GpuProblem::HostDevUnavailable)?;
        let mut nodes = Vec::new();
        if sys.join("class/kfd/kfd").exists() {
            nodes.push(dev.join("kfd"));
            probed.devices.push("/dev/kfd".to_owned());
        }
        nodes.extend(device_nodes(&dev.join("dri"), langs)?);
        probed.devices.push("/dev/dri".to_owned());
        probed.groups = owning_groups(&nodes)?;
        probed.vendors.extend(
            render_vendors
                .iter()
                .filter_map(|vendor| requirement_vendor(vendor)),
        );
    }
    if nvidia {
        probed.vendors.insert(GpuVendor::Nvidia);
    }
    Ok(probed)
}

/// PCI 厂商号对应的清单厂商；核心不认识的为空。
fn requirement_vendor(id: &str) -> Option<GpuVendor> {
    match id {
        "0x1002" => Some(GpuVendor::Amd),
        NVIDIA_VENDOR => Some(GpuVendor::Nvidia),
        "0x8086" => Some(GpuVendor::Intel),
        _ => None,
    }
}

const NVIDIA_VENDOR: &str = "0x10de";

fn vendor_name(id: &str, langs: &Languages) -> String {
    match id {
        "0x1002" => "AMD".to_owned(),
        NVIDIA_VENDOR => "NVIDIA".to_owned(),
        "0x8086" => "Intel".to_owned(),
        other => text(
            langs,
            "plugin-install-gpu-vendor-other",
            &[("id", other.into())],
        ),
    }
}

/// `/sys/class/drm` 里的渲染节点（`renderD128` 等）。目录不存在就是没有 DRM 设备。
fn render_nodes(sys: &Path) -> Result<Vec<String>, GpuProblem> {
    let mut nodes: Vec<String> = entries(&sys.join("class/drm"))?
        .into_iter()
        .filter(|name| name.starts_with("renderD"))
        .collect();
    nodes.sort();
    Ok(nodes)
}

/// 没有渲染节点也没有 NVIDIA 驱动时，从 PCI 设备里找出显卡，说明为什么用不了。
fn no_usable_driver(sys: &Path, langs: &Languages) -> Result<GpuProblem, GpuProblem> {
    let pci = sys.join("bus/pci/devices");
    let mut cards = Vec::new();
    for address in entries(&pci)? {
        let device = pci.join(&address);
        if !read_trimmed(&device.join("class")).starts_with("0x03") {
            continue;
        }
        let vendor = vendor_name(&read_trimmed(&device.join("vendor")), langs);
        let driver = fs::read_link(device.join("driver"))
            .ok()
            .and_then(|link| link.file_name().map(|n| n.to_string_lossy().into_owned()));
        let state = match driver.as_deref() {
            None => text(langs, "plugin-install-gpu-no-driver-none", &[]),
            Some("nouveau") => text(langs, "plugin-install-gpu-no-driver-nouveau", &[]),
            Some(driver) => text(
                langs,
                "plugin-install-gpu-no-driver-no-render",
                &[("driver", driver.into())],
            ),
        };
        cards.push(text(
            langs,
            "plugin-install-gpu-device-entry",
            &[
                ("address", address.as_str().into()),
                ("vendor", vendor.as_str().into()),
                ("state", state.as_str().into()),
            ],
        ));
    }
    if cards.is_empty() {
        Ok(GpuProblem::NoDevice)
    } else {
        Ok(GpuProblem::NoDriver(list(langs, &cards)))
    }
}

/// 目录里的设备节点（不算子目录，如 `/dev/dri/by-path`）。目录不存在时报错：sysfs 说有设备，`/dev` 里却没有。
fn device_nodes(dir: &Path, langs: &Languages) -> Result<Vec<PathBuf>, GpuProblem> {
    let nodes: Vec<PathBuf> = entries(dir)?
        .into_iter()
        .map(|name| dir.join(name))
        .filter(|path| !path.is_dir())
        .collect();
    if nodes.is_empty() {
        return Err(GpuProblem::Unreadable {
            path: dir.display().to_string(),
            error: text(langs, "plugin-install-gpu-no-device-nodes", &[]),
        });
    }
    Ok(nodes)
}

/// 设备节点所属的组，去重排序；root 组（0）不用加。
fn owning_groups(nodes: &[PathBuf]) -> Result<Vec<u32>, GpuProblem> {
    let mut groups = BTreeSet::new();
    for node in nodes {
        let meta = fs::metadata(node).map_err(|e| GpuProblem::Unreadable {
            path: node.display().to_string(),
            error: e.to_string(),
        })?;
        if meta.gid() != 0 {
            groups.insert(meta.gid());
        }
    }
    Ok(groups.into_iter().collect())
}

/// 目录里的名字，按字节序；目录不存在时为空，别的错误如实报。
fn entries(dir: &Path) -> Result<Vec<String>, GpuProblem> {
    let read = match fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(GpuProblem::Unreadable {
                path: dir.display().to_string(),
                error: e.to_string(),
            });
        }
    };
    let mut names: Vec<String> = read
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    Ok(names)
}

pub(super) fn read_trimmed(path: &Path) -> String {
    fs::read_to_string(path)
        .map(|text| text.trim().to_owned())
        .unwrap_or_default()
}

/// 安装预览里关于显卡的部分：一行说明（拿显卡的服务，本机会给什么或者为什么给不了），以及 `required` 而本机给不了时
/// 装不了的原因，和其他“这台主机上装不了”的原因一样放进 `InstallPreview.blocked`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuPreview {
    pub notice: Option<CardLine>,
    pub blocked: Option<String>,
}

impl GpuPreview {
    /// 没申请显卡、或可选权限关着。
    pub fn none() -> Self {
        Self {
            notice: None,
            blocked: None,
        }
    }

    pub fn line(line: String, blocked: Option<String>) -> Self {
        Self {
            notice: Some(CardLine {
                text: line,
                emphasis: true,
            }),
            blocked,
        }
    }
}

#[cfg(test)]
pub(crate) use super::gpu_fake_host as fake_host;
