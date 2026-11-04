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
use serde::{Deserialize, Serialize};
use wonton_protocol::CardLine;

use super::host_check;
use super::manager::{InstallError, PluginManager};
use super::manifest::{GpuNeed, Manifest, ManifestError, Permission};
use super::requirements::{GpuVendor, card_name};

/// 清单的 `gpu`：拿到显卡的服务。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuSpec {
    pub services: Vec<String>,
}

/// `gpu`、`devices.gpu` 和 `requirements.gpu` 三者要一起出现：只写其中一个说明清单写错了，而不是不要显卡。
/// 服务名是否存在由 [`super::package`] 对照 compose 检查。
pub fn validate(manifest: &Manifest) -> Result<(), ManifestError> {
    let invalid = |message: &str| Err(ManifestError::Invalid(message.to_owned()));
    let permitted = manifest.permissions.contains(&Permission::DevicesGpu);
    let wants = manifest.requirements.as_ref().and_then(|r| r.gpu.as_ref());
    match (&manifest.gpu, permitted) {
        (Some(_), false) => return invalid("声明了 gpu 就必须申请 devices.gpu 权限"),
        (None, true) => {
            return invalid("申请了 devices.gpu 就必须用 gpu.services 写明哪些服务用显卡");
        }
        (None, false) if wants.is_some() => {
            return invalid("requirements.gpu 要和 devices.gpu 权限、gpu.services 一起声明");
        }
        _ => {}
    }
    if let Some(gpu) = &manifest.gpu {
        let mut seen = BTreeSet::new();
        if gpu.services.is_empty() || !gpu.services.iter().all(|s| seen.insert(s)) {
            return invalid("gpu.services 至少一项，且不能重复");
        }
    }
    Ok(())
}

/// 插件要不要显卡：没申请 `devices.gpu` 的不要；申请了而没写 `requirements.gpu` 的按 `required`。
pub fn requirement(manifest: &Manifest) -> Option<GpuNeed> {
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
    fn only(mut self, wanted: &[GpuVendor]) -> Result<Self, GpuProblem> {
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
            return Err(GpuProblem::NoWantedVendor(card_name(wanted)));
        }
        Ok(self)
    }

    /// 给主人看的一句话：给了什么。
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if !self.devices.is_empty() {
            let vendors: Vec<&str> = self
                .vendors
                .iter()
                .filter(|v| *v != "NVIDIA")
                .map(String::as_str)
                .collect();
            let groups: Vec<String> = self.groups.iter().map(ToString::to_string).collect();
            parts.push(format!(
                "{} 显卡：{}，容器加入组 {}",
                vendors.join("、"),
                self.devices.join("、"),
                groups.join("、")
            ));
        }
        if self.nvidia {
            parts.push("NVIDIA 显卡：全部，经 NVIDIA Container Toolkit".to_owned());
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

/// 没有可用的显卡的原因，给主人看。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GpuProblem {
    #[error(
        "核心在容器里，但没有挂载宿主的 /dev（WONTON_HOST_DEV），看不到显卡设备：按当前版本的 compose 重新启动核心"
    )]
    HostDevUnavailable,
    #[error("读不了 {path}：{error}")]
    Unreadable { path: String, error: String },
    #[error("没有找到显卡（PCI 上没有显示设备）")]
    NoDevice,
    #[error("有显卡但没有可用的驱动：{0}")]
    NoDriver(String),
    #[error(
        "有 NVIDIA 驱动，但 Docker 没有登记 nvidia 运行时：请安装 NVIDIA Container Toolkit 并执行 \
         `nvidia-ctk runtime configure --runtime=docker`"
    )]
    NoRuntime,
    #[error("没有{0}")]
    NoWantedVendor(String),
}

/// 按 sysfs 和宿主的 `/dev` 探测显卡。NVIDIA 只确认驱动已加载（`nvidia` 内核模块），运行时由调用方对照 Docker
/// 再查；`nvidia-smi` 之类的二进制装了不算，驱动没加载时它也在。
pub fn probe(paths: &HostPaths) -> Result<GpuGrant, GpuProblem> {
    let sys = &paths.sys;
    let nvidia = sys.join("module/nvidia").is_dir();
    let render_vendors: Vec<String> = render_nodes(sys)?
        .iter()
        .map(|node| read_trimmed(&sys.join("class/drm").join(node).join("device/vendor")))
        .filter(|vendor| vendor != NVIDIA_VENDOR)
        .collect();
    if render_vendors.is_empty() && !nvidia {
        return Err(no_usable_driver(sys)?);
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
        nodes.extend(device_nodes(&dev.join("dri"))?);
        grant.devices.push("/dev/dri".to_owned());
        grant.groups = owning_groups(&nodes)?;
        let vendors: BTreeSet<String> = render_vendors.iter().map(|v| vendor_name(v)).collect();
        grant.vendors.extend(vendors);
    }
    if nvidia {
        grant.vendors.push("NVIDIA".to_owned());
    }
    Ok(grant)
}

const NVIDIA_VENDOR: &str = "0x10de";

fn vendor_name(id: &str) -> String {
    match id {
        "0x1002" => "AMD".to_owned(),
        NVIDIA_VENDOR => "NVIDIA".to_owned(),
        "0x8086" => "Intel".to_owned(),
        other => format!("厂商 {other}"),
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
fn no_usable_driver(sys: &Path) -> Result<GpuProblem, GpuProblem> {
    let pci = sys.join("bus/pci/devices");
    let mut cards = Vec::new();
    for address in entries(&pci)? {
        let device = pci.join(&address);
        if !read_trimmed(&device.join("class")).starts_with("0x03") {
            continue;
        }
        let vendor = vendor_name(&read_trimmed(&device.join("vendor")));
        let driver = fs::read_link(device.join("driver"))
            .ok()
            .and_then(|link| link.file_name().map(|n| n.to_string_lossy().into_owned()));
        let state = match driver.as_deref() {
            None => "没有加载驱动".to_owned(),
            Some("nouveau") => "用的是 nouveau 驱动，不能用于计算，要 NVIDIA 官方驱动".to_owned(),
            Some(driver) => format!("加载了 {driver} 驱动，但没有渲染节点"),
        };
        cards.push(format!("{address}（{vendor}）{state}"));
    }
    if cards.is_empty() {
        Ok(GpuProblem::NoDevice)
    } else {
        Ok(GpuProblem::NoDriver(cards.join("；")))
    }
}

/// 目录里的设备节点（不算子目录，如 `/dev/dri/by-path`）。目录不存在时报错：sysfs 说有设备，`/dev` 里却没有。
fn device_nodes(dir: &Path) -> Result<Vec<PathBuf>, GpuProblem> {
    let nodes: Vec<PathBuf> = entries(dir)?
        .into_iter()
        .map(|name| dir.join(name))
        .filter(|path| !path.is_dir())
        .collect();
    if nodes.is_empty() {
        return Err(GpuProblem::Unreadable {
            path: dir.display().to_string(),
            error: "里面没有设备节点".to_owned(),
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
    ) -> Result<Result<GpuGrant, GpuProblem>, InstallError> {
        let requirements = manifest.requirements.as_ref();
        let wanted = requirements
            .and_then(|r| r.gpu.as_ref())
            .map_or(&[][..], |gpu| &gpu.vendors);
        let grant = match probe(&self.host).and_then(|grant| grant.only(wanted)) {
            Ok(grant) => grant,
            Err(problem) => return Ok(Err(problem)),
        };
        if grant.nvidia && !self.engine.runtimes().await?.iter().any(|r| r == "nvidia") {
            return Ok(Err(GpuProblem::NoRuntime));
        }
        Ok(Ok(grant))
    }

    /// 给插件的显卡，或者给不了的原因：先按它的要求判断（厂商、显存），再看设备实际能不能交出去。
    async fn gpu_fit(&self, manifest: &Manifest) -> Result<Result<GpuGrant, String>, InstallError> {
        if let Some(problem) = host_check::gpu_problem(manifest, &self.host_facts().await) {
            return Ok(Err(problem));
        }
        Ok(self.probe_gpu(manifest).await?.map_err(|p| p.to_string()))
    }

    /// 申请了 `devices.gpu` 的插件在这台主机上拿到什么：没有符合的显卡时，`required` 的装不了（原因写清），
    /// `optional` 的照常装、不给设备。没申请的返回 `None`，不碰 Docker。
    pub(super) async fn gpu_grant(
        &self,
        manifest: &Manifest,
    ) -> Result<Option<GpuGrant>, InstallError> {
        let Some(requirement) = requirement(manifest) else {
            return Ok(None);
        };
        match self.gpu_fit(manifest).await? {
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
    ) -> Result<GpuPreview, InstallError> {
        let Some(requirement) = requirement(manifest) else {
            return Ok(GpuPreview {
                notice: None,
                blocked: None,
            });
        };
        let services = manifest
            .gpu
            .as_ref()
            .map(|g| g.services.join("、"))
            .unwrap_or_default();
        let (text, blocked) = match self.gpu_fit(manifest).await? {
            Ok(grant) => (
                format!("显卡交给服务 {services}：{}", grant.describe()),
                None,
            ),
            Err(problem) if requirement == GpuNeed::Optional => (
                format!("本机没有符合的显卡（{problem}），服务 {services} 装上后不带显卡运行"),
                None,
            ),
            Err(problem) => {
                let reason = InstallError::GpuUnavailable(problem).to_string();
                (format!("服务 {services} 需要显卡，{reason}"), Some(reason))
            }
        };
        Ok(GpuPreview {
            notice: Some(CardLine {
                text,
                emphasis: true,
            }),
            blocked,
        })
    }
}

#[cfg(test)]
pub(crate) mod fake_host {
    //! 测试用的假宿主：在临时目录里摆出 sysfs 和 `/dev` 的相关部分。设备节点用普通文件代替（测试进程不能
    //! mknod），所属的组就是测试进程的组。

    use std::fs;
    use std::path::Path;

    use super::HostPaths;

    pub struct FakeHost {
        pub dir: tempfile::TempDir,
    }

    impl FakeHost {
        pub fn empty() -> Self {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(dir.path().join("sys/class/drm")).unwrap();
            fs::create_dir_all(dir.path().join("sys/bus/pci/devices")).unwrap();
            fs::create_dir_all(dir.path().join("dev")).unwrap();
            Self { dir }
        }

        /// npchy 那样的 AMD 主机：amdgpu 驱动、KFD、一个渲染节点和一个显示节点。
        pub fn amd() -> Self {
            let host = Self::empty();
            host.pci("0000:05:00.0", "0x1002", "0x038000", Some("amdgpu"));
            host.render_node("renderD128", "0x1002", "0000:05:00.0");
            fs::create_dir_all(host.sys().join("class/kfd/kfd")).unwrap();
            host.dev_node("kfd");
            host.dev_node("dri/card1");
            host.dev_node("dri/renderD128");
            fs::create_dir_all(host.dev().join("dri/by-path")).unwrap();
            host
        }

        /// 加载了 NVIDIA 官方驱动的主机：有 `nvidia` 内核模块，渲染节点属于 NVIDIA。
        pub fn nvidia() -> Self {
            let host = Self::empty();
            host.pci("0000:01:00.0", "0x10de", "0x030000", Some("nvidia"));
            host.render_node("renderD128", "0x10de", "0000:01:00.0");
            fs::create_dir_all(host.sys().join("module/nvidia")).unwrap();
            host
        }

        pub fn sys(&self) -> std::path::PathBuf {
            self.dir.path().join("sys")
        }

        pub fn dev(&self) -> std::path::PathBuf {
            self.dir.path().join("dev")
        }

        /// 核心直接跑在宿主上：看得到 `/dev`。
        pub fn paths(&self) -> HostPaths {
            HostPaths {
                dev: Some(self.dev()),
                sys: self.sys(),
                proc: self.sys().with_file_name("proc"),
            }
        }

        /// 核心在容器里，compose 没有挂载宿主的 `/dev`。
        pub fn paths_without_dev(&self) -> HostPaths {
            HostPaths {
                dev: None,
                sys: self.sys(),
                proc: self.sys().with_file_name("proc"),
            }
        }

        pub fn pci(&self, address: &str, vendor: &str, class: &str, driver: Option<&str>) {
            let device = self.sys().join("bus/pci/devices").join(address);
            fs::create_dir_all(&device).unwrap();
            fs::write(device.join("vendor"), format!("{vendor}\n")).unwrap();
            fs::write(device.join("class"), format!("{class}\n")).unwrap();
            if let Some(driver) = driver {
                let drivers = self.sys().join("bus/pci/drivers").join(driver);
                fs::create_dir_all(&drivers).unwrap();
                std::os::unix::fs::symlink(&drivers, device.join("driver")).unwrap();
            }
        }

        pub fn render_node(&self, name: &str, vendor: &str, address: &str) {
            let node = self.sys().join("class/drm").join(name);
            fs::create_dir_all(&node).unwrap();
            let device = self.sys().join("bus/pci/devices").join(address);
            std::os::unix::fs::symlink(&device, node.join("device")).unwrap();
            fs::create_dir_all(device.join("drm").join(name)).unwrap();
            fs::write(device.join("vendor"), format!("{vendor}\n")).unwrap();
        }

        /// `/proc/meminfo`，只有总内存。
        pub fn memory_total_kib(&self, kib: u64) {
            let proc = self.sys().with_file_name("proc");
            fs::create_dir_all(&proc).unwrap();
            fs::write(proc.join("meminfo"), format!("MemTotal: {kib} kB\n")).unwrap();
        }

        pub fn dev_node(&self, relative: &str) {
            let path = self.dev().join(relative);
            fs::create_dir_all(path.parent().unwrap_or(Path::new("/"))).unwrap();
            fs::write(path, "").unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::fake_host::FakeHost;
    use super::*;

    #[test]
    fn amd_hosts_hand_over_kfd_dri_and_the_owning_groups() {
        let host = FakeHost::amd();
        let grant = probe(&host.paths()).unwrap();
        assert_eq!(grant.devices, ["/dev/kfd", "/dev/dri"]);
        assert!(!grant.nvidia);
        assert_eq!(grant.vendors, ["AMD"]);
        // 假设备节点是普通文件，属于测试进程的组；真机上是 video、render 的 GID。
        let gid = std::fs::metadata(host.dev().join("kfd")).unwrap().gid();
        let expected: Vec<u32> = if gid == 0 { Vec::new() } else { vec![gid] };
        assert_eq!(grant.groups, expected);
        let text = grant.describe();
        assert!(text.starts_with("AMD 显卡：/dev/kfd、/dev/dri"), "{text}");
    }

    #[test]
    fn render_nodes_without_kfd_still_get_dri() {
        let host = FakeHost::empty();
        host.pci("0000:00:02.0", "0x8086", "0x030000", Some("i915"));
        host.render_node("renderD128", "0x8086", "0000:00:02.0");
        host.dev_node("dri/renderD128");
        let grant = probe(&host.paths()).unwrap();
        assert_eq!(grant.devices, ["/dev/dri"]);
        assert_eq!(grant.vendors, ["Intel"]);
    }

    #[test]
    fn nvidia_needs_the_kernel_module_not_the_binary() {
        let host = FakeHost::nvidia();
        let grant = probe(&host.paths()).unwrap();
        assert!(grant.nvidia);
        assert!(grant.devices.is_empty(), "NVIDIA 的设备由运行时放进容器");
        assert_eq!(grant.vendors, ["NVIDIA"]);
        assert_eq!(
            grant.describe(),
            "NVIDIA 显卡：全部，经 NVIDIA Container Toolkit"
        );

        // 同样的显卡，驱动没加载（nvidia-smi 装了也没用）：如实说没有可用的驱动。
        let host = FakeHost::empty();
        host.pci("0000:01:00.0", "0x10de", "0x030000", None);
        let err = probe(&host.paths()).unwrap_err();
        assert!(
            matches!(&err, GpuProblem::NoDriver(detail) if detail.contains("NVIDIA") && detail.contains("没有加载驱动")),
            "{err}"
        );
        let host = FakeHost::empty();
        host.pci("0000:01:00.0", "0x10de", "0x030000", Some("nouveau"));
        host.render_node("renderD128", "0x10de", "0000:01:00.0");
        let err = probe(&host.paths()).unwrap_err();
        assert!(
            matches!(&err, GpuProblem::NoDriver(detail) if detail.contains("nouveau")),
            "{err}"
        );
    }

    #[test]
    fn hosts_without_a_gpu_say_so() {
        let host = FakeHost::empty();
        host.pci("0000:00:1f.0", "0x8086", "0x060100", Some("lpc_ich"));
        assert_eq!(probe(&host.paths()).unwrap_err(), GpuProblem::NoDevice);
        // sysfs 里连 drm、pci 目录都没有（比如测试机）也是没有显卡，不是读错。
        let bare = tempfile::tempdir().unwrap();
        let paths = HostPaths {
            dev: Some(bare.path().join("dev")),
            sys: bare.path().join("sys"),
            proc: bare.path().join("proc"),
        };
        assert_eq!(probe(&paths).unwrap_err(), GpuProblem::NoDevice);
    }

    #[test]
    fn a_core_in_a_container_without_the_host_dev_mount_does_not_guess() {
        let host = FakeHost::amd();
        assert_eq!(
            probe(&host.paths_without_dev()).unwrap_err(),
            GpuProblem::HostDevUnavailable
        );
        // 有渲染节点、/dev/dri 里却没有设备节点：报读不了，不悄悄给一个空目录。
        let host = FakeHost::empty();
        host.pci("0000:05:00.0", "0x1002", "0x038000", Some("amdgpu"));
        host.render_node("renderD128", "0x1002", "0000:05:00.0");
        std::fs::create_dir_all(host.dev().join("dri")).unwrap();
        assert!(matches!(
            probe(&host.paths()).unwrap_err(),
            GpuProblem::Unreadable { .. }
        ));
    }

    #[test]
    fn grants_become_devices_groups_and_device_requests() {
        let grant = GpuGrant {
            devices: vec!["/dev/kfd".into(), "/dev/dri".into()],
            groups: vec![983, 987],
            nvidia: true,
            vendors: vec!["AMD".into(), "NVIDIA".into()],
        };
        let mut host = HostConfig::default();
        grant.apply(&mut host);
        let devices = host.devices.unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].path_on_host.as_deref(), Some("/dev/kfd"));
        assert_eq!(devices[0].path_in_container.as_deref(), Some("/dev/kfd"));
        assert_eq!(devices[0].cgroup_permissions.as_deref(), Some("rwm"));
        assert_eq!(host.group_add.unwrap(), ["983", "987"]);
        let request = &host.device_requests.unwrap()[0];
        assert_eq!(request.count, Some(-1));
        assert_eq!(
            request.capabilities.as_deref(),
            Some(&[vec!["gpu".to_owned()]][..])
        );
        assert_eq!(
            grant.describe(),
            "AMD 显卡：/dev/kfd、/dev/dri，容器加入组 983、987；NVIDIA 显卡：全部，经 NVIDIA Container Toolkit"
        );
    }
}
