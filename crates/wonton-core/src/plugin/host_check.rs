//! 这台主机满不满足插件的要求：拿清单的 `requirements` 对照 [`HostFacts`]。查不出来是 `Unknown`，不当成满足，也不拦
//! 安装；不满足的 `required` 项才拦。只在商店、预览和安装时判断，插件装上以后主机变了不重新判断。

use wonton_protocol::{RequirementLine, RequirementStatus as Status};

use super::docker::HostInfo;
use super::host_facts::{self, ContainerAccess, Gpu, HostFacts, Known};
use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::manifest::Manifest;
use super::requirements::{self, Arch, GpuNeed, GpuRequirement, Requirements, parse_size};
use crate::l10n::{list, text};

impl PluginManager {
    /// 这台主机实际有什么。Docker 不可达时相关的项是查不出来，不报错。
    pub(super) async fn host_facts(&self, langs: &Languages) -> HostFacts {
        let docker = self.engine.host_info().await.map_err(|e| e.to_string());
        // 往下一路是 Docker 的 hyper 请求链，调试构建的帧很大：放堆上跑，别让它挤占调用方的栈。
        Box::pin(self.facts_from(docker, langs)).await
    }

    pub(super) async fn facts_from(
        &self,
        docker: Result<HostInfo, String>,
        langs: &Languages,
    ) -> HostFacts {
        let mut facts = host_facts::read(&self.host, docker, langs);
        let core = self.cfg.core_container.as_deref();
        self.nvidia_memory
            .fill(&mut facts, &self.engine, core, &self.cfg.instance, langs)
            .await;
        facts
    }

    /// 主机总内存，读不出来时为空。只拿数字，不写给人看的原因，不用语言。
    pub(super) async fn memory_total(&self) -> Option<u64> {
        let docker = self.engine.host_info().await.map_err(|e| e.to_string());
        match host_facts::read(&self.host, docker, &Languages::default()).memory_total {
            Known::Known(total) => Some(total),
            Known::Unknown(_) => None,
        }
    }

    /// 宿主主机名（Docker `/info` 的 `Name`：daemon 跑在宿主上，安装时登记的名字就是宿主
    /// hostname，核心自己的 hostname 不算数）。取到才缓存，之后不再查 Docker；失败打 warn、
    /// 这次注入空串、不缓存，下次安装再取——失败即停的例外：主机名只是显示用（插件拿它填
    /// `hostname`、信息条），注入空串是如实报告没有这个值，不是伪装成功。
    pub async fn host_name(&self) -> String {
        match self
            .host_name
            .get_or_try_init(|| async {
                match self.engine.host_info().await {
                    Ok(info) => Ok(info.name.unwrap_or_default()),
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            "can't get the host hostname; injecting an empty WONTON_HOST_NAME, retrying on the next install"
                        );
                        Err(error.to_string())
                    }
                }
            })
            .await
        {
            Ok(name) => name.clone(),
            Err(_) => String::new(),
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
    fn refusal(&self, langs: &Languages) -> Option<InstallError> {
        if self.line.status != Status::Unmet {
            return None;
        }
        let problem = self.line.problem.as_deref().unwrap_or_default();
        let joined = |why: &str| {
            text(
                langs,
                "plugin-install-host-gpu-problem",
                &[
                    ("card", self.line.text.as_str().into()),
                    ("why", why.into()),
                ],
            )
        };
        Some(match self.item {
            Item::Gpu => InstallError::GpuUnavailable(problem.to_owned()),
            Item::Arch | Item::Memory => InstallError::Unsupported(joined(problem)),
        })
    }
}

pub fn judge(requirements: &Requirements, facts: &HostFacts, langs: &Languages) -> Vec<Check> {
    let mut checks = Vec::new();
    if let Some(wanted) = &requirements.arch {
        let text = requirements::arch_line(wanted, langs);
        checks.push(Check::new(Item::Arch, text, arch(wanted, facts, langs)));
    }
    if let Some(wanted) = &requirements.memory {
        let text = requirements::memory_line(wanted, langs);
        checks.push(Check::new(Item::Memory, text, memory(wanted, facts, langs)));
    }
    if let Some(wanted) = &requirements.gpu {
        let text = requirements::gpu_line(wanted, langs);
        checks.push(Check::new(Item::Gpu, text, gpu(wanted, facts, langs)));
    }
    checks
}

/// 清单里第一项不满足的要求报的错；都满足或者查不出来时为空。
pub fn refusal(manifest: &Manifest, facts: &HostFacts, langs: &Languages) -> Option<InstallError> {
    let requirements = manifest.requirements.as_ref()?;
    judge(requirements, facts, langs)
        .iter()
        .find_map(|check| check.refusal(langs))
}

/// 显卡要求不满足时为什么：`required` 的装不了，`optional` 的不给设备。其余情况（满足、查不出来）为空，由实际探测设备决定
/// 给什么。没写 `requirements.gpu` 时等于 `need: required`。
pub fn gpu_problem(manifest: &Manifest, facts: &HostFacts, langs: &Languages) -> Option<String> {
    let requirements = manifest.requirements.as_ref();
    let wanted = requirements.and_then(|r| r.gpu.clone()).unwrap_or_default();
    match gpu(&wanted, facts, langs) {
        (Status::Unmet | Status::Degraded, problem) => problem,
        _ => None,
    }
}

type Verdict = (Status, Option<String>);

fn unknown(langs: &Languages, what: &str, why: &str) -> Verdict {
    let problem = text(
        langs,
        "plugin-install-host-unknown",
        &[("what", what.into()), ("why", why.into())],
    );
    (Status::Unknown, Some(problem))
}

fn arch(wanted: &[Arch], facts: &HostFacts, langs: &Languages) -> Verdict {
    let what = text(langs, "plugin-install-host-arch-unknown-what", &[]);
    match &facts.arch {
        Known::Unknown(why) => unknown(langs, &what, why),
        Known::Known(arch) => match Arch::from_docker(arch) {
            Some(host) if wanted.contains(&host) => (Status::Met, None),
            _ => {
                let problem = text(
                    langs,
                    "plugin-install-host-arch-mismatch",
                    &[("arch", arch.as_str().into())],
                );
                (Status::Unmet, Some(problem))
            }
        },
    }
}

/// 标称 8G 的机器实际报出来不到 8G，显卡也一样（标称 8G 的卡报 8188 MiB），所以到要求的 90% 就算够。
fn enough(have: u64, need: u64) -> bool {
    u128::from(have) * 10 >= u128::from(need) * 9
}

fn memory(wanted: &str, facts: &HostFacts, langs: &Languages) -> Verdict {
    let Some(need) = parse_size(wanted) else {
        let what = text(langs, "plugin-install-host-memory-wanted-what", &[]);
        return unknown(langs, &what, wanted);
    };
    let what = text(langs, "plugin-install-host-memory-unknown-what", &[]);
    match &facts.memory_total {
        Known::Unknown(why) => unknown(langs, &what, why),
        Known::Known(total) if enough(*total, need) => (Status::Met, None),
        Known::Known(total) => {
            let problem = text(
                langs,
                "plugin-install-host-memory-low",
                &[("size", size(*total).into())],
            );
            (Status::Unmet, Some(problem))
        }
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

fn gpu(wanted: &GpuRequirement, facts: &HostFacts, langs: &Languages) -> Verdict {
    let what = text(langs, "plugin-install-host-gpu-unknown-what", &[]);
    let gpus = match &facts.gpus {
        Known::Unknown(why) => return unknown(langs, &what, why),
        Known::Known(gpus) => gpus,
    };
    let candidates: Vec<&Gpu> = gpus
        .iter()
        .filter(|gpu| wanted.vendors.is_empty() || wanted.vendors.iter().any(|v| gpu.vendor.is(*v)))
        .collect();
    let fits: Vec<(&Gpu, Fit)> = candidates
        .iter()
        .map(|gpu| (*gpu, fit(gpu, wanted, langs)))
        .collect();
    if fits.iter().any(|(_, fit)| matches!(fit, Fit::Yes)) {
        return (Status::Met, None);
    }
    let problem_line = |gpu: &Gpu, why: &str| {
        text(
            langs,
            "plugin-install-host-gpu-problem",
            &[("card", gpu.name(langs).into()), ("why", why.into())],
        )
    };
    let maybe: Vec<String> = fits
        .iter()
        .filter_map(|(gpu, fit)| match fit {
            Fit::Maybe(why) => Some(problem_line(gpu, why)),
            _ => None,
        })
        .collect();
    if !maybe.is_empty() {
        return (Status::Unknown, Some(list(langs, &maybe)));
    }
    // 有能用的卡只是显存不够时，不提那些本来就用不了的（如服务器的管理显卡）。
    let usable = fits
        .iter()
        .any(|(gpu, _)| matches!(gpu.in_containers, ContainerAccess::Usable { .. }));
    let no: Vec<String> = fits
        .iter()
        .filter(|(gpu, _)| !usable || matches!(gpu.in_containers, ContainerAccess::Usable { .. }))
        .filter_map(|(gpu, fit)| match fit {
            Fit::No(why) => Some(problem_line(gpu, why)),
            _ => None,
        })
        .collect();
    let problem = if no.is_empty() {
        let card = requirements::card_name(&wanted.vendors, langs);
        text(
            langs,
            "plugin-install-host-gpu-missing",
            &[("card", card.as_str().into())],
        )
    } else {
        list(langs, &no)
    };
    let status = match wanted.need {
        GpuNeed::Required => Status::Unmet,
        GpuNeed::Optional => Status::Degraded,
    };
    (status, Some(problem))
}

fn fit(gpu: &Gpu, wanted: &GpuRequirement, langs: &Languages) -> Fit {
    match &gpu.in_containers {
        ContainerAccess::Unusable(why) => return Fit::No(why.clone()),
        ContainerAccess::Unknown(why) => return Fit::Maybe(why.clone()),
        ContainerAccess::Usable { .. } => {}
    }
    let Some(wanted_text) = wanted.memory.as_deref() else {
        return Fit::Yes;
    };
    let Some(need) = parse_size(wanted_text) else {
        return Fit::Maybe(text(
            langs,
            "plugin-install-host-fit-vram-unreadable",
            &[("text", wanted_text.into())],
        ));
    };
    match &gpu.vram {
        Known::Unknown(why) => Fit::Maybe(text(
            langs,
            "plugin-install-host-fit-unknown",
            &[("why", why.as_str().into())],
        )),
        Known::Known(vram) if enough(*vram, need) => Fit::Yes,
        Known::Known(vram) if gpu.shared_memory => Fit::Maybe(text(
            langs,
            "plugin-install-host-fit-integrated",
            &[("size", size(*vram).into())],
        )),
        Known::Known(vram) => Fit::No(text(
            langs,
            "plugin-install-host-fit-vram-low",
            &[("size", size(*vram).into()), ("text", wanted_text.into())],
        )),
    }
}
