//! 这台主机满不满足插件的要求：拿清单的 `requirements` 对照 [`HostFacts`]。查不出来是 `Unknown`，不当成满足，也不拦
//! 安装；不满足的 `required` 项才拦。只在商店、预览和安装时判断，插件装上以后主机变了不重新判断。

use wonton_protocol::{RequirementLine, RequirementStatus as Status};

use super::docker::HostInfo;
use super::host_facts::{self, ContainerAccess, Gpu, HostFacts, Known};
use super::manager::{InstallError, PluginManager};
use super::manifest::Manifest;
use super::requirements::{self, Arch, GpuNeed, GpuRequirement, Requirements, parse_size};

impl PluginManager {
    /// 这台主机实际有什么。Docker 不可达时相关的项是查不出来，不报错。
    pub(super) async fn host_facts(&self) -> HostFacts {
        let docker = self.engine.host_info().await.map_err(|e| e.to_string());
        self.facts_from(docker).await
    }

    pub(super) async fn facts_from(&self, docker: Result<HostInfo, String>) -> HostFacts {
        let mut facts = host_facts::read(&self.host, docker);
        let core = self.cfg.core_container.as_deref();
        self.nvidia_memory
            .fill(&mut facts, &self.engine, core, &self.cfg.instance)
            .await;
        facts
    }

    /// 主机总内存，读不出来时为空。
    pub(super) async fn memory_total(&self) -> Option<u64> {
        let docker = self.engine.host_info().await.map_err(|e| e.to_string());
        match host_facts::read(&self.host, docker).memory_total {
            Known::Known(total) => Some(total),
            Known::Unknown(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Arch,
    Memory,
    Gpu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub item: Item,
    pub line: RequirementLine,
}

impl Check {
    fn new(item: Item, text: String, (status, problem): (Status, Option<String>)) -> Self {
        Self {
            item,
            line: RequirementLine {
                text,
                status,
                problem,
            },
        }
    }

    /// 不满足时安装报的错：架构和内存是 `unsupported`，显卡是 `gpu_unavailable`。
    fn refusal(&self) -> Option<InstallError> {
        if self.line.status != Status::Unmet {
            return None;
        }
        let problem = self.line.problem.as_deref().unwrap_or_default();
        Some(match self.item {
            Item::Gpu => InstallError::GpuUnavailable(problem.to_owned()),
            Item::Arch | Item::Memory => {
                InstallError::Unsupported(format!("{}：{problem}", self.line.text))
            }
        })
    }
}

pub fn judge(requirements: &Requirements, facts: &HostFacts) -> Vec<Check> {
    let mut checks = Vec::new();
    if let Some(wanted) = &requirements.arch {
        let text = requirements::arch_line(wanted);
        checks.push(Check::new(Item::Arch, text, arch(wanted, facts)));
    }
    if let Some(wanted) = &requirements.memory {
        let text = requirements::memory_line(wanted);
        checks.push(Check::new(Item::Memory, text, memory(wanted, facts)));
    }
    if let Some(wanted) = &requirements.gpu {
        let text = requirements::gpu_line(wanted);
        checks.push(Check::new(Item::Gpu, text, gpu(wanted, facts)));
    }
    checks
}

/// 清单里第一项不满足的要求报的错；都满足或者查不出来时为空。
pub fn refusal(manifest: &Manifest, facts: &HostFacts) -> Option<InstallError> {
    let requirements = manifest.requirements.as_ref()?;
    judge(requirements, facts).iter().find_map(Check::refusal)
}

/// 显卡要求不满足时为什么：`required` 的装不了，`optional` 的不给设备。其余情况（满足、查不出来）为空，由实际探测设备决定
/// 给什么。没写 `requirements.gpu` 时等于 `need: required`。
pub fn gpu_problem(manifest: &Manifest, facts: &HostFacts) -> Option<String> {
    let requirements = manifest.requirements.as_ref();
    let wanted = requirements.and_then(|r| r.gpu.clone()).unwrap_or_default();
    match gpu(&wanted, facts) {
        (Status::Unmet | Status::Degraded, problem) => problem,
        _ => None,
    }
}

type Verdict = (Status, Option<String>);

fn unknown(what: &str, why: &str) -> Verdict {
    (Status::Unknown, Some(format!("查不出{what}：{why}")))
}

fn arch(wanted: &[Arch], facts: &HostFacts) -> Verdict {
    match &facts.arch {
        Known::Unknown(why) => unknown("这台主机的架构", why),
        Known::Known(arch) => match Arch::from_docker(arch) {
            Some(host) if wanted.contains(&host) => (Status::Met, None),
            _ => (Status::Unmet, Some(format!("这台主机是 {arch}"))),
        },
    }
}

/// 标称 8G 的机器实际报出来不到 8G，显卡也一样（标称 8G 的卡报 8188 MiB），所以到要求的 90% 就算够。
fn enough(have: u64, need: u64) -> bool {
    u128::from(have) * 10 >= u128::from(need) * 9
}

fn memory(wanted: &str, facts: &HostFacts) -> Verdict {
    let Some(need) = parse_size(wanted) else {
        return unknown("要多少内存", wanted);
    };
    match &facts.memory_total {
        Known::Unknown(why) => unknown("这台主机的内存", why),
        Known::Known(total) if enough(*total, need) => (Status::Met, None),
        Known::Known(total) => (
            Status::Unmet,
            Some(format!("这台主机只有 {} 内存", size(*total))),
        ),
    }
}

/// 给人看的大小，如 `1.5 GB`、`512 MB`。
pub(super) fn size(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;
    if bytes < GIB {
        return format!("{} MB", bytes / MIB);
    }
    let tenths = bytes * 10 / GIB;
    if tenths.is_multiple_of(10) {
        format!("{} GB", tenths / 10)
    } else {
        format!("{}.{} GB", tenths / 10, tenths % 10)
    }
}

enum Fit {
    Yes,
    Maybe(String),
    No(String),
}

fn gpu(wanted: &GpuRequirement, facts: &HostFacts) -> Verdict {
    let gpus = match &facts.gpus {
        Known::Unknown(why) => return unknown("这台主机的显卡", why),
        Known::Known(gpus) => gpus,
    };
    let candidates: Vec<&Gpu> = gpus
        .iter()
        .filter(|gpu| wanted.vendors.is_empty() || wanted.vendors.iter().any(|v| gpu.vendor.is(*v)))
        .collect();
    let fits: Vec<(&Gpu, Fit)> = candidates
        .iter()
        .map(|gpu| (*gpu, fit(gpu, wanted)))
        .collect();
    if fits.iter().any(|(_, fit)| matches!(fit, Fit::Yes)) {
        return (Status::Met, None);
    }
    let maybe: Vec<String> = fits
        .iter()
        .filter_map(|(gpu, fit)| match fit {
            Fit::Maybe(why) => Some(format!("{}：{why}", gpu.name())),
            _ => None,
        })
        .collect();
    if !maybe.is_empty() {
        return (Status::Unknown, Some(maybe.join("；")));
    }
    // 有能用的卡只是显存不够时，不提那些本来就用不了的（如服务器的管理显卡）。
    let usable = fits
        .iter()
        .any(|(gpu, _)| matches!(gpu.in_containers, ContainerAccess::Usable { .. }));
    let no: Vec<String> = fits
        .iter()
        .filter(|(gpu, _)| !usable || matches!(gpu.in_containers, ContainerAccess::Usable { .. }))
        .filter_map(|(gpu, fit)| match fit {
            Fit::No(why) => Some(format!("{}：{why}", gpu.name())),
            _ => None,
        })
        .collect();
    let problem = if no.is_empty() {
        format!("这台主机没有{}", requirements::card_name(&wanted.vendors))
    } else {
        no.join("；")
    };
    let status = match wanted.need {
        GpuNeed::Required => Status::Unmet,
        GpuNeed::Optional => Status::Degraded,
    };
    (status, Some(problem))
}

fn fit(gpu: &Gpu, wanted: &GpuRequirement) -> Fit {
    match &gpu.in_containers {
        ContainerAccess::Unusable(why) => return Fit::No(why.clone()),
        ContainerAccess::Unknown(why) => return Fit::Maybe(why.clone()),
        ContainerAccess::Usable { .. } => {}
    }
    let Some(text) = wanted.memory.as_deref() else {
        return Fit::Yes;
    };
    let Some(need) = parse_size(text) else {
        return Fit::Maybe(format!("读不懂显存要求 {text}"));
    };
    match &gpu.vram {
        Known::Unknown(why) => Fit::Maybe(format!("查不出显存：{why}")),
        Known::Known(vram) if enough(*vram, need) => Fit::Yes,
        Known::Known(vram) if gpu.shared_memory => Fit::Maybe(format!(
            "集成显卡和内存共用，划给它的 {} 不是上限，够不够查不出来",
            size(*vram)
        )),
        Known::Known(vram) => Fit::No(format!("显存只有 {}，要 {text}", size(*vram))),
    }
}
