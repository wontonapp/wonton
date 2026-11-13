//! 把宿主的显卡交给插件（ADR-0018）。清单的 `gpu.services` 说明哪些服务用显卡，`devices.gpu` 权限由主人确认；
//! 交给容器什么由核心按宿主实际有的显卡决定，不收插件 compose 里自己写的 `devices`。
//!
//! 探测只看文件：sysfs 是内核的，核心在容器里看到的和宿主上一样，由它判断有没有显卡、加载了什么驱动；设备节点
//! 和它们所属的组在宿主的 `/dev` 里，核心在容器里时要由 compose 只读挂载进来（`WONTON_HOST_DEV`）。NVIDIA 的
//! 设备不由核心列举，交给 Docker 的 `DeviceRequests`（NVIDIA Container Toolkit），核心只确认驱动和运行时都在。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use bollard::models::{DeviceMapping, DeviceRequest, HostConfig};
use serde::{Deserialize, Serialize};
use wonton_protocol::CardLine;

use super::host_check;
use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::manifest::{GpuImages, GpuNeed, Manifest, ManifestError, Permission};
use super::requirements::{GpuVendor, card_name};
use crate::l10n::{list, text};

/// 清单的 `gpu`：拿到显卡的服务。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuSpec {
    pub services: Vec<String>,
    /// 按厂商换的镜像（ADR-0099），键是 `services` 里的服务。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub images: GpuImages,
}

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
    /// sysfs。测试里换成假的目录树。
    pub sys: PathBuf,
    /// procfs，同上。
    pub proc: PathBuf,
}

impl HostPaths {
    /// 核心直接跑在宿主上时就是 `/dev`；在容器里时要配置挂载进来的路径（`WONTON_HOST_DEV`）。
    pub fn new(host_dev: Option<PathBuf>) -> Self {
        Self {
            dev: host_dev,
            sys: PathBuf::from("/sys"),
            proc: PathBuf::from("/proc"),
        }
    }
}

/// 交给服务的显卡。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuGrant {
    /// 宿主上的设备路径，原样出现在容器里：`/dev/dri`（整个目录，含渲染节点），有 KFD 时还有 `/dev/kfd`。
    pub devices: Vec<String>,
    /// 这些设备节点所属的组的 GID（通常是宿主的 `video`、`render`），容器加入它们，非 root 的进程才开得了设备。
    pub groups: Vec<u32>,
    /// 全部 NVIDIA 显卡：经 Docker 的 `DeviceRequests` 由 NVIDIA Container Toolkit 放进容器，同 `--gpus all`。
    pub nvidia: bool,
    /// 显卡的厂商，给人看。
    pub vendors: Vec<String>,
}

impl GpuGrant {
    /// 只留插件要的厂商；空表示哪家都行。`/dev/dri` 是整个目录交出去的（理由见 ADR-0018），AMD 和 Intel 混装的主机上
    /// 分不开，所以任一家要的卡在 DRM 上就给整个目录。
    fn only(mut self, wanted: &[GpuVendor], langs: &Languages) -> Result<Self, GpuProblem> {
        if wanted.is_empty() {
            return Ok(self);
        }
        let names: Vec<&str> = wanted.iter().map(|vendor| vendor.name()).collect();
        self.nvidia &= wanted.contains(&GpuVendor::Nvidia);
        let on_drm = self
            .vendors
            .iter()
            .any(|vendor| vendor != "NVIDIA" && names.contains(&vendor.as_str()));
        if !on_drm {
            self.devices.clear();
            self.groups.clear();
        }
        self.vendors
            .retain(|vendor| names.contains(&vendor.as_str()));
        if !self.nvidia && self.devices.is_empty() {
            return Err(GpuProblem::NoWantedVendor(card_name(wanted, langs)));
        }
        Ok(self)
    }

    /// 给主人看的一句话：给了什么。
    pub fn describe(&self, langs: &Languages) -> String {
        let mut parts = Vec::new();
        if !self.devices.is_empty() {
            let vendors: Vec<&str> = self
                .vendors
                .iter()
                .filter(|v| *v != "NVIDIA")
                .map(String::as_str)
                .collect();
            let groups: Vec<String> = self.groups.iter().map(ToString::to_string).collect();
            parts.push(text(
                langs,
                "plugin-install-gpu-describe-vendors",
                &[
                    ("vendors", list(langs, &vendors).into()),
                    ("devices", list(langs, &self.devices).into()),
                    ("groups", list(langs, &groups).into()),
                ],
            ));
        }
        if self.nvidia {
            parts.push(text(langs, "plugin-install-gpu-describe-nvidia", &[]));
        }
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
    Unreadable { path: String, error: String },
    NoDevice,
    NoDriver(String),
    NoRuntime,
    NoWantedVendor(String),
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
            Self::NoWantedVendor(card) => text(
                langs,
                "plugin-install-gpu-problem-no-wanted-vendor",
                &[("card", card.as_str().into())],
            ),
        }
    }
}

/// 按 sysfs 和宿主的 `/dev` 探测显卡。NVIDIA 只确认驱动已加载（`nvidia` 内核模块），运行时由调用方对照 Docker
/// 再查；`nvidia-smi` 之类的二进制装了不算，驱动没加载时它也在。
pub fn probe(paths: &HostPaths, langs: &Languages) -> Result<GpuGrant, GpuProblem> {
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
    let mut grant = GpuGrant {
        devices: Vec::new(),
        groups: Vec::new(),
        nvidia,
        vendors: Vec::new(),
    };
    if !render_vendors.is_empty() {
        let dev = paths.dev.as_ref().ok_or(GpuProblem::HostDevUnavailable)?;
        let mut nodes = Vec::new();
        if sys.join("class/kfd/kfd").exists() {
            nodes.push(dev.join("kfd"));
            grant.devices.push("/dev/kfd".to_owned());
        }
        nodes.extend(device_nodes(&dev.join("dri"), langs)?);
        grant.devices.push("/dev/dri".to_owned());
        grant.groups = owning_groups(&nodes)?;
        let vendors: BTreeSet<String> = render_vendors
            .iter()
            .map(|v| vendor_name(v, langs))
            .collect();
        grant.vendors.extend(vendors);
    }
    if nvidia {
        grant.vendors.push("NVIDIA".to_owned());
    }
    Ok(grant)
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

impl PluginManager {
    /// 插件要的厂商的显卡：文件里看驱动和设备，Docker 里看 NVIDIA 的运行时。Docker 不可达照常报错。
    async fn probe_gpu(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<Result<GpuGrant, GpuProblem>, InstallError> {
        let requirements = manifest.requirements.as_ref();
        let wanted = requirements
            .and_then(|r| r.gpu.as_ref())
            .map_or(&[][..], |gpu| &gpu.vendors);
        let grant = match probe(&self.host, langs).and_then(|grant| grant.only(wanted, langs)) {
            Ok(grant) => grant,
            Err(problem) => return Ok(Err(problem)),
        };
        if grant.nvidia && !self.engine.runtimes().await?.iter().any(|r| r == "nvidia") {
            return Ok(Err(GpuProblem::NoRuntime));
        }
        Ok(Ok(grant))
    }

    /// 给插件的显卡，或者给不了的原因：先按它的要求判断（厂商、显存），再看设备实际能不能交出去。
    async fn gpu_fit(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<Result<GpuGrant, String>, InstallError> {
        if let Some(problem) =
            host_check::gpu_problem(manifest, &self.host_facts(langs).await, langs)
        {
            return Ok(Err(problem));
        }
        Ok(self
            .probe_gpu(manifest, langs)
            .await?
            .map_err(|p| p.message(langs)))
    }

    /// 申请了 `devices.gpu` 的插件在这台主机上拿到什么：没有符合的显卡时，`required` 的装不了（原因写清），
    /// `optional` 的照常装、不给设备。没申请的返回 `None`，不碰 Docker。
    pub(super) async fn gpu_grant(
        &self,
        manifest: &Manifest,
        granted: &[Permission],
        langs: &Languages,
    ) -> Result<Option<GpuGrant>, InstallError> {
        let Some(requirement) = requirement(manifest) else {
            return Ok(None);
        };
        if !granted.contains(&Permission::DevicesGpu) {
            return Ok(None);
        }
        match self.gpu_fit(manifest, langs).await? {
            Ok(grant) => Ok(Some(grant)),
            Err(problem) if requirement == GpuNeed::Optional => {
                tracing::info!(plugin = %manifest.id, %problem, "本机没有符合的显卡，插件不带显卡安装");
                Ok(None)
            }
            Err(problem) => Err(InstallError::GpuUnavailable(problem)),
        }
    }

    /// 没申请 `devices.gpu` 的插件没有这部分，也不碰 Docker。
    pub(super) async fn gpu_preview(
        &self,
        manifest: &Manifest,
        enabled: bool,
        langs: &Languages,
    ) -> Result<GpuPreview, InstallError> {
        let Some(requirement) = requirement(manifest) else {
            return Ok(GpuPreview {
                notice: None,
                blocked: None,
            });
        };
        if !enabled {
            return Ok(GpuPreview {
                notice: None,
                blocked: None,
            });
        }
        let services = manifest
            .gpu
            .as_ref()
            .map(|g| list(langs, &g.services))
            .unwrap_or_default();
        let (line, blocked) = match self.gpu_fit(manifest, langs).await? {
            Ok(grant) => (
                text(
                    langs,
                    "plugin-install-gpu-preview-granted",
                    &[
                        ("services", services.as_str().into()),
                        ("describe", grant.describe(langs).into()),
                    ],
                ),
                None,
            ),
            Err(problem) if requirement == GpuNeed::Optional => (
                text(
                    langs,
                    "plugin-install-gpu-preview-missing",
                    &[
                        ("problem", problem.as_str().into()),
                        ("services", services.as_str().into()),
                    ],
                ),
                None,
            ),
            Err(problem) => {
                let reason = InstallError::GpuUnavailable(problem).to_string();
                (
                    text(
                        langs,
                        "plugin-install-gpu-preview-required",
                        &[
                            ("services", services.as_str().into()),
                            ("reason", reason.as_str().into()),
                        ],
                    ),
                    Some(reason),
                )
            }
        };
        Ok(GpuPreview {
            notice: Some(CardLine {
                text: line,
                emphasis: true,
            }),
            blocked,
        })
    }
}

#[cfg(test)]
pub(crate) use super::gpu_fake_host as fake_host;
