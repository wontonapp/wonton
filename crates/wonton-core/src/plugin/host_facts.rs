//! 主机实际有什么：内存、系统、架构、显卡，给判断插件的“要求”用。读不出来的如实是 [`Known::Unknown`] 并写明原因，
//! 不猜，也不当成有。
//!
//! 内存读 `/proc/meminfo`：容器里看到的也是宿主的，不受容器的内存限制影响。显卡读 sysfs，理由见 [`super::gpu`]。
//! 实测（2026-11，Linux 6.11.0-29-generic 和 7.2.5-3-omarchy）：
//! - 一台机器可能有好几张“显示设备”：带 BMC 的服务器有 ASPEED 管理显卡（`0x1a03`，有 `card0`、没有渲染节点），算不了
//!   东西，要和真正的显卡分开。
//! - `/usr/bin/nvidia-smi` 装了不代表有驱动，只看内核驱动绑定到了哪张卡。
//! - 型号只有 NVIDIA 驱动在 `/proc` 里写明；AMD 有的代有 `product_name`，其余只有设备 ID，要 `pci.ids` 才翻得出名字，
//!   核心不带它。显存 AMD、Intel 独显在 sysfs 里有，NVIDIA 没有，见 [`NvidiaMemory`]。
//! - AMD 集成显卡的 `mem_info_vram_total` 是划给它的那部分内存，不是上限（npchy 的 0x1586 报 64 GiB，有的 APU 只报
//!   512 MB）；独显有 `mem_info_vram_vendor`，集显没有。

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use bollard::models::{ContainerCreateBody, DeviceRequest, HostConfig};
use serde::Serialize;

use super::docker::{Engine, HostInfo};
use super::gpu::{HostPaths, read_trimmed};
use super::names::Instance;
use super::requirements;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Known<T> {
    Known(T),
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostFacts {
    /// Docker 报的 `OSType`，如 `linux`。
    pub os: Known<String>,
    /// Docker 报的架构，如 `x86_64`。
    pub arch: Known<String>,
    /// 字节。
    pub memory_total: Known<u64>,
    /// 字节，内核估计的不用换页就能拿到的内存（`MemAvailable`）。
    pub memory_available: Known<u64>,
    pub gpus: Known<Vec<Gpu>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    /// 其他厂商的 PCI ID，如 `0x1a03`。
    Other(String),
}

impl GpuVendor {
    pub fn is(&self, wanted: requirements::GpuVendor) -> bool {
        matches!(
            (self, wanted),
            (Self::Nvidia, requirements::GpuVendor::Nvidia)
                | (Self::Amd, requirements::GpuVendor::Amd)
                | (Self::Intel, requirements::GpuVendor::Intel)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gpu {
    /// PCI 地址，如 `0000:c7:00.0`。
    pub address: String,
    pub vendor: GpuVendor,
    /// PCI 设备 ID，如 `0x2b85`。
    pub device: String,
    /// 绑定的内核驱动。
    pub driver: Option<String>,
    pub model: Known<String>,
    /// 显存，字节。
    pub vram: Known<u64>,
    /// AMD 集成显卡：`vram` 是划给它的那部分内存，不是上限。
    pub shared_memory: bool,
    /// 插件的容器能不能用上这张卡。
    pub in_containers: ContainerAccess,
}

impl Gpu {
    /// 给人看的名字：有型号写型号，没有就是厂商和设备 ID。
    pub fn name(&self) -> String {
        if let Known::Known(model) = &self.model {
            return model.clone();
        }
        match &self.vendor {
            GpuVendor::Nvidia => format!("NVIDIA {}", self.device),
            GpuVendor::Amd => format!("AMD {}", self.device),
            GpuVendor::Intel => format!("Intel {}", self.device),
            GpuVendor::Other(vendor) => format!("显示设备 {vendor}:{}", self.device),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerAccess {
    /// 能用，这些是交给容器的设备节点（NVIDIA 由运行时给，为空）。
    Usable {
        devices: Vec<String>,
    },
    /// 用不了，写明缺什么。
    Unusable(String),
    Unknown(String),
}

/// `docker` 是向 Docker 查询的结果，查不到时写明原因。
pub fn read(paths: &HostPaths, docker: Result<HostInfo, String>) -> HostFacts {
    let (os, arch, runtimes) = match docker {
        Ok(info) => (
            reported(info.os_type),
            reported(info.arch),
            Known::Known(info.runtimes),
        ),
        Err(reason) => (
            Known::Unknown(reason.clone()),
            Known::Unknown(reason.clone()),
            Known::Unknown(reason),
        ),
    };
    let meminfo = fs::read_to_string(paths.proc.join("meminfo"));
    let memory = |field: &str| match &meminfo {
        Ok(text) => meminfo_bytes(text, field).map_or_else(
            || Known::Unknown(format!("/proc/meminfo 里没有 {field}")),
            Known::Known,
        ),
        Err(e) => Known::Unknown(format!("读不了 /proc/meminfo：{e}")),
    };
    HostFacts {
        os,
        arch,
        memory_total: memory("MemTotal"),
        memory_available: memory("MemAvailable"),
        gpus: read_gpus(paths, &runtimes),
    }
}

fn reported(value: String) -> Known<String> {
    if value.is_empty() {
        Known::Unknown("Docker 没有报告".to_owned())
    } else {
        Known::Known(value)
    }
}

fn meminfo_bytes(text: &str, field: &str) -> Option<u64> {
    let line = text
        .lines()
        .find_map(|l| l.strip_prefix(field)?.strip_prefix(':'))?;
    let kib: u64 = line.trim().strip_suffix("kB")?.trim().parse().ok()?;
    kib.checked_mul(1024)
}

fn read_gpus(paths: &HostPaths, runtimes: &Known<Vec<String>>) -> Known<Vec<Gpu>> {
    let pci = paths.sys.join("bus/pci/devices");
    let addresses = match fs::read_dir(&pci) {
        Ok(read) => read,
        Err(e) => return Known::Unknown(format!("读不了 {}：{e}", pci.display())),
    };
    let mut addresses: Vec<String> = addresses
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    addresses.sort();
    Known::Known(
        addresses
            .iter()
            .filter(|address| read_trimmed(&pci.join(address).join("class")).starts_with("0x03"))
            .map(|address| read_gpu(paths, address, runtimes))
            .collect(),
    )
}

fn read_gpu(paths: &HostPaths, address: &str, runtimes: &Known<Vec<String>>) -> Gpu {
    let device = paths.sys.join("bus/pci/devices").join(address);
    let vendor = match read_trimmed(&device.join("vendor")).as_str() {
        "0x10de" => GpuVendor::Nvidia,
        "0x1002" => GpuVendor::Amd,
        "0x8086" => GpuVendor::Intel,
        other => GpuVendor::Other(other.to_owned()),
    };
    let driver = fs::read_link(device.join("driver"))
        .ok()
        .and_then(|link| link.file_name().map(|n| n.to_string_lossy().into_owned()));
    let in_containers = match vendor {
        GpuVendor::Nvidia => nvidia_access(driver.as_deref(), runtimes),
        _ => render_access(paths, &device, driver.as_deref()),
    };
    Gpu {
        address: address.to_owned(),
        model: model(paths, address, &vendor, &device),
        vram: vram(&vendor, &device),
        shared_memory: vendor == GpuVendor::Amd && !device.join("mem_info_vram_vendor").exists(),
        device: read_trimmed(&device.join("device")),
        vendor,
        driver,
        in_containers,
    }
}

fn nvidia_access(driver: Option<&str>, runtimes: &Known<Vec<String>>) -> ContainerAccess {
    match driver {
        Some("nvidia") => {}
        Some("nouveau") => {
            let reason = "用的是 nouveau 驱动，不能用于计算，要 NVIDIA 官方驱动";
            return ContainerAccess::Unusable(reason.to_owned());
        }
        Some(other) => return ContainerAccess::Unusable(format!("绑定的是 {other} 驱动")),
        None => return ContainerAccess::Unusable("没有加载驱动".to_owned()),
    }
    match runtimes {
        Known::Known(list) if list.iter().any(|r| r == "nvidia") => ContainerAccess::Usable {
            devices: Vec::new(),
        },
        Known::Known(_) => ContainerAccess::Unusable(
            "Docker 没有登记 nvidia 运行时：要安装 NVIDIA Container Toolkit".to_owned(),
        ),
        Known::Unknown(reason) => {
            ContainerAccess::Unknown(format!("不知道 Docker 有没有 nvidia 运行时：{reason}"))
        }
    }
}

fn render_access(paths: &HostPaths, device: &Path, driver: Option<&str>) -> ContainerAccess {
    let Some(driver) = driver else {
        return ContainerAccess::Unusable("没有加载驱动".to_owned());
    };
    let render = fs::read_dir(device.join("drm")).ok().and_then(|read| {
        read.filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .find(|name| name.starts_with("renderD"))
    });
    let Some(render) = render else {
        return ContainerAccess::Unusable(format!("{driver} 驱动没有渲染节点，不能用于计算"));
    };
    let Some(dev) = &paths.dev else {
        return ContainerAccess::Unknown(
            "核心在容器里，但没有挂载宿主的 /dev（WONTON_HOST_DEV），看不到设备节点".to_owned(),
        );
    };
    if !dev.join("dri").join(&render).exists() {
        return ContainerAccess::Unusable(format!("宿主的 /dev/dri 里没有 {render}"));
    }
    let mut devices = vec![format!("/dev/dri/{render}")];
    if driver == "amdgpu" && dev.join("kfd").exists() {
        devices.push("/dev/kfd".to_owned());
    }
    ContainerAccess::Usable { devices }
}

fn model(paths: &HostPaths, address: &str, vendor: &GpuVendor, device: &Path) -> Known<String> {
    let found = match vendor {
        GpuVendor::Nvidia => {
            let information = paths
                .proc
                .join("driver/nvidia/gpus")
                .join(address)
                .join("information");
            fs::read_to_string(information).ok().and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("Model:"))
                    .map(|model| model.trim().to_owned())
            })
        }
        _ => Some(read_trimmed(&device.join("product_name"))),
    };
    match found.filter(|model| !model.is_empty()) {
        Some(model) => Known::Known(model),
        None => Known::Unknown("驱动没有给出型号，只知道设备 ID".to_owned()),
    }
}

fn vram(vendor: &GpuVendor, device: &Path) -> Known<u64> {
    let found = match vendor {
        GpuVendor::Amd => read_trimmed(&device.join("mem_info_vram_total"))
            .parse()
            .ok(),
        GpuVendor::Intel => local_memory(device),
        GpuVendor::Nvidia => {
            return Known::Unknown("NVIDIA 的显存不在 sysfs 里".to_owned());
        }
        GpuVendor::Other(_) => None,
    };
    found.map_or_else(
        || Known::Unknown("驱动没有给出显存大小".to_owned()),
        Known::Known,
    )
}

/// Intel 独显的显存在 `drm/cardN/lmem_total_bytes`，集显没有这个文件。
fn local_memory(device: &Path) -> Option<u64> {
    let cards = fs::read_dir(device.join("drm")).ok()?;
    cards.filter_map(Result::ok).find_map(|entry| {
        read_trimmed(&entry.path().join("lmem_total_bytes"))
            .parse()
            .ok()
    })
}

const MIB: u64 = 1024 * 1024;
/// 问失败了隔多久再问：起容器要几秒，商店和预览不能每次都等。问到的不用再问，驱动不重启显存不变。
const ASK_AGAIN: Duration = Duration::from_mins(1);

type NvidiaAnswer = Result<Vec<(String, u64)>, String>;

/// NVIDIA 的显存只有驱动知道：用核心的镜像起一次性容器，用插件拿显卡时一样的 `DeviceRequests`，运行时把 `nvidia-smi`
/// 带进去（`NVIDIA_DRIVER_CAPABILITIES=utility`），问它。问到的卡填上显存；问不到时写明原因，容器里能不能用上也改成
/// 不确定：运行时登记了不等于真能用。容器名是固定的，同时只问一次，其他的等着用它的结果。
#[derive(Default)]
pub struct NvidiaMemory(tokio::sync::Mutex<Option<(NvidiaAnswer, Instant)>>);

impl NvidiaMemory {
    pub async fn fill(
        &self,
        facts: &mut HostFacts,
        engine: &Engine,
        core: Option<&str>,
        instance: &Instance,
    ) {
        let Known::Known(gpus) = &mut facts.gpus else {
            return;
        };
        let mut nvidia: Vec<&mut Gpu> = gpus
            .iter_mut()
            .filter(|gpu| gpu.vendor == GpuVendor::Nvidia)
            .filter(|gpu| matches!(gpu.in_containers, ContainerAccess::Usable { .. }))
            .collect();
        if nvidia.is_empty() {
            return;
        }
        let Some(core) = core else {
            let reason = "核心不在容器里，没有镜像可以用来问 NVIDIA 驱动";
            for gpu in &mut nvidia {
                gpu.vram = Known::Unknown(reason.to_owned());
            }
            return;
        };
        match self.answer(engine, core, instance).await {
            Ok(cards) => {
                for gpu in &mut nvidia {
                    if let Some((_, memory)) = cards.iter().find(|(a, _)| *a == gpu.address) {
                        gpu.vram = Known::Known(*memory);
                    }
                }
            }
            Err(reason) => {
                for gpu in &mut nvidia {
                    gpu.vram = Known::Unknown(reason.clone());
                    gpu.in_containers =
                        ContainerAccess::Unknown(format!("在容器里试用失败：{reason}"));
                }
            }
        }
    }

    async fn answer(&self, engine: &Engine, core: &str, instance: &Instance) -> NvidiaAnswer {
        let mut asked = self.0.lock().await;
        if let Some((answer, at)) = &*asked
            && (answer.is_ok() || at.elapsed() < ASK_AGAIN)
        {
            return answer.clone();
        }
        let answer =
            run_nvidia_smi(engine, core, &instance.gpu_probe(), instance.core_labels()).await;
        *asked = Some((answer.clone(), Instant::now()));
        answer
    }
}

async fn run_nvidia_smi(
    engine: &Engine,
    core: &str,
    name: &str,
    labels: HashMap<String, String>,
) -> Result<Vec<(String, u64)>, String> {
    let core_info = engine
        .inspect_container(core)
        .await
        .map_err(|e| e.to_string())?;
    let Some(image) = core_info.and_then(|c| c.image) else {
        return Err(format!("查不到核心容器 {core} 的镜像"));
    };
    let body = ContainerCreateBody {
        image: Some(image),
        entrypoint: Some(vec!["nvidia-smi".to_owned()]),
        cmd: Some(vec![
            "--query-gpu=pci.bus_id,memory.total".to_owned(),
            "--format=csv,noheader,nounits".to_owned(),
        ]),
        env: Some(vec!["NVIDIA_DRIVER_CAPABILITIES=utility".to_owned()]),
        labels: Some(labels),
        host_config: Some(HostConfig {
            network_mode: Some("none".to_owned()),
            cap_drop: Some(vec!["ALL".to_owned()]),
            security_opt: Some(vec!["no-new-privileges".to_owned()]),
            device_requests: Some(vec![DeviceRequest {
                count: Some(-1),
                capabilities: Some(vec![vec!["gpu".to_owned()]]),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let output = engine
        .run_once(name, body)
        .await
        .map_err(|e| e.to_string())??;
    if output.code != 0 {
        let reason = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "nvidia-smi 以 {} 退出：{}",
            output.code,
            reason.trim()
        ));
    }
    Ok(parse_nvidia_smi(&String::from_utf8_lossy(&output.stdout)))
}

/// `00000000:C7:00.0, 32607`：地址换成 sysfs 的写法（四位域、小写），显存是 MiB。读不懂的行不算。
fn parse_nvidia_smi(text: &str) -> Vec<(String, u64)> {
    text.lines()
        .filter_map(|line| {
            let (address, memory) = line.split_once(',')?;
            let (domain, rest) = address.trim().split_once(':')?;
            let domain = u32::from_str_radix(domain, 16).ok()?;
            let memory: u64 = memory.trim().parse().ok()?;
            let address = format!("{domain:04x}:{}", rest.to_lowercase());
            Some((address, memory.checked_mul(MIB)?))
        })
        .collect()
}
