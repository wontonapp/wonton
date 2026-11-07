//! 插件服务的 Docker 资源限制，以及插件总量的 systemd slice。

use bollard::models::{ContainerUpdateBody, HostConfig};
use wonton_protocol::PluginLimits;

use super::compose::{Scalar, Service, parse_bytes};
use super::languages::Languages;
use super::manager::InstallError;
use super::requirements::{Requirements, parse_size};

const DEFAULT_CPU_SHARES: i64 = 512;
const DEFAULT_PIDS_LIMIT: i64 = 4096;
const OOM_SCORE_ADJ: i64 = 500;
const GIB: u64 = 1 << 30;
const NANO_CPUS: f64 = 1_000_000_000.0;
const NANO_CPUS_PER_PERCENT: i64 = 10_000_000;

struct Resolved {
    memory: Option<i64>,
    memory_reservation: Option<i64>,
    nano_cpus: Option<i64>,
}

/// 主人的值优先于 compose，compose 优先于默认值。
fn resolve(
    service: &Service,
    requirements: Option<&Requirements>,
    memory_total: Option<u64>,
    owner: &PluginLimits,
    langs: &Languages,
) -> Result<Resolved, String> {
    let compose_memory = positive_bytes(langs, "mem_limit", service.mem_limit.as_ref())?;
    let compose_reservation =
        positive_bytes(langs, "mem_reservation", service.mem_reservation.as_ref())?;
    if let (Some(limit), Some(reservation)) = (compose_memory, compose_reservation)
        && reservation > limit
    {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-limits-mem-reservation-order",
            &[],
        ));
    }
    let memory = match owner.memory {
        Some(value) => Some(value),
        None => memory_limit(compose_memory, requirements, memory_total, langs)?,
    };
    // Docker 拒绝比 `Memory` 大的 `MemoryReservation`。
    let reservation =
        compose_reservation.map(|value| memory.map_or(value, |limit| value.min(limit)));
    let nano_cpus = owner
        .cpu_percent
        .map(|percent| i64::from(percent) * NANO_CPUS_PER_PERCENT)
        .or(service
            .cpus
            .as_ref()
            .map(|value| parse_cpus(value, langs))
            .transpose()?);
    Ok(Resolved {
        memory: memory.map(|value| to_i64(langs, value)).transpose()?,
        memory_reservation: reservation.map(|value| to_i64(langs, value)).transpose()?,
        nano_cpus,
    })
}

/// 把容器从 `current` 改成 `want`（渲染出的 `HostConfig`）要发的 `POST /containers/{id}/update`。
///
/// Docker 29.7.2 实测：`Memory`、`NanoCpus` 写 0 都是“不改”，清不掉；`Memory: -1` 被拒。
/// 所以内存上限清不掉，只能等重建。CPU 上限改成全部核数，`cpu.max` 实际不限，下次重建回到不写；
/// 不用 `CpuQuota: -1`：cgroup 变成不限，但 `HostConfig.NanoCpus` 还是旧值，两边对不上。
pub fn update_body(
    want: &HostConfig,
    current: &HostConfig,
    ncpu: i64,
    langs: &Languages,
) -> Result<ContainerUpdateBody, InstallError> {
    let limited = |value: Option<i64>| value.is_some_and(|value| value > 0);
    if want.memory.is_none() && limited(current.memory) {
        return Err(InstallError::Unsupported(crate::l10n::text(
            langs,
            "plugin-install-limits-host-memory-unknown",
            &[],
        )));
    }
    let nano_cpus = match want.nano_cpus {
        None if limited(current.nano_cpus) => {
            let all = ncpu.checked_mul(NANO_CPUS as i64);
            Some(all.ok_or_else(|| {
                InstallError::Docker(crate::l10n::text(
                    langs,
                    "plugin-install-limits-ncpu-bad",
                    &[("ncpu", ncpu.into())],
                ))
            })?)
        }
        nano_cpus => nano_cpus,
    };
    Ok(ContainerUpdateBody {
        memory: want.memory,
        memory_swap: want.memory,
        memory_reservation: want.memory_reservation,
        nano_cpus,
        ..Default::default()
    })
}

/// 把一个服务声明的限制和宿主默认值写进 Docker 的 `HostConfig`。
pub fn apply(
    host: &mut HostConfig,
    service: &Service,
    requirements: Option<&Requirements>,
    memory_total: Option<u64>,
    cgroup_parent: Option<&str>,
    owner: &PluginLimits,
    langs: &Languages,
) -> Result<(), String> {
    let resolved = resolve(service, requirements, memory_total, owner, langs)?;
    host.cpu_shares = Some(
        positive_integer(langs, "cpu_shares", service.cpu_shares.as_ref())?
            .unwrap_or(DEFAULT_CPU_SHARES),
    );
    host.memory = resolved.memory;
    host.memory_swap = resolved.memory;
    host.memory_reservation = resolved.memory_reservation;
    host.nano_cpus = resolved.nano_cpus;
    host.pids_limit = Some(
        positive_integer(langs, "pids_limit", service.pids_limit.as_ref())?
            .unwrap_or(DEFAULT_PIDS_LIMIT),
    );
    host.oom_score_adj = Some(OOM_SCORE_ADJ);
    host.cgroup_parent = cgroup_parent
        .filter(|parent| !parent.is_empty())
        .map(str::to_owned);
    Ok(())
}

fn memory_limit(
    compose_memory: Option<u64>,
    requirements: Option<&Requirements>,
    memory_total: Option<u64>,
    langs: &Languages,
) -> Result<Option<u64>, String> {
    if compose_memory.is_some() {
        return Ok(compose_memory);
    }
    let Some(total) = memory_total else {
        return Ok(None);
    };
    let eighty_percent = total / 5 * 4 + (total % 5 * 4) / 5;
    let required = requirements
        .and_then(|r| r.memory.as_deref())
        .map(|value| {
            parse_size(value).ok_or_else(|| {
                crate::l10n::text(
                    langs,
                    "plugin-install-limits-memory-format",
                    &[("value", value.into())],
                )
            })
        })
        .transpose()?
        .unwrap_or(0);
    Ok(Some(eighty_percent.max(required).min(total)))
}

fn positive_bytes(
    langs: &Languages,
    field: &str,
    value: Option<&Scalar>,
) -> Result<Option<u64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let parsed = parse_bytes(value, langs).map_err(|message| forward(langs, field, message))?;
    if parsed <= 0 {
        return Err(positive(langs, field));
    }
    Ok(Some(parsed as u64))
}

fn positive_integer(
    langs: &Languages,
    field: &str,
    value: Option<&Scalar>,
) -> Result<Option<i64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let parsed = match value {
        Scalar::Int(value) => *value,
        _ => {
            return Err(crate::l10n::text(
                langs,
                "plugin-install-limits-integer-positive",
                &[("field", field.into())],
            ));
        }
    };
    if parsed <= 0 {
        return Err(positive(langs, field));
    }
    Ok(Some(parsed))
}

fn parse_cpus(value: &Scalar, langs: &Languages) -> Result<i64, String> {
    let text = value.as_string();
    let parsed: f64 = text.parse().map_err(|_| {
        crate::l10n::text(
            langs,
            "plugin-install-limits-cpus-format",
            &[("text", text.as_str().into())],
        )
    })?;
    if !parsed.is_finite() || parsed <= 0.0 {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-limits-cpus-positive",
            &[],
        ));
    }
    let nanos = parsed * NANO_CPUS;
    if !nanos.is_finite() || nanos < 1.0 || nanos > i64::MAX as f64 {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-limits-cpus-range",
            &[("text", text.as_str().into())],
        ));
    }
    Ok(nanos.round() as i64)
}

fn to_i64(langs: &Languages, value: u64) -> Result<i64, String> {
    i64::try_from(value)
        .map_err(|_| crate::l10n::text(langs, "plugin-install-limits-memory-range", &[]))
}

/// `{ $field }：{ $error }`（plugin-install-limits-field-forward）。
fn forward(langs: &Languages, field: &str, error: String) -> String {
    crate::l10n::text(
        langs,
        "plugin-install-limits-field-forward",
        &[("field", field.into()), ("error", error.into())],
    )
}

fn positive(langs: &Languages, field: &str) -> String {
    crate::l10n::text(
        langs,
        "plugin-install-limits-positive",
        &[("field", field.into())],
    )
}

/// 从 `/proc/meminfo` 读取 `MemTotal`，单位转换为字节。
pub fn parse_mem_total(text: &str) -> Result<u64, String> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .ok_or_else(|| "读取不到 /proc/meminfo 的 MemTotal".to_owned())?;
    let kib = value
        .trim()
        .strip_suffix("kB")
        .ok_or_else(|| "MemTotal 的单位不是 kB".to_owned())?
        .trim()
        .parse::<u64>()
        .map_err(|_| "MemTotal 不是合法的整数".to_owned())?;
    kib.checked_mul(1024)
        .filter(|bytes| *bytes > 0)
        .ok_or_else(|| "MemTotal 超出字节数范围".to_owned())
}

/// 根据宿主总内存生成插件 systemd slice 的 unit 文本。不设 `MemoryHigh`：实测它把 slice 里所有插件卡住，
/// 失控的进程却不被杀，见 ADR-0037 的“实测”。
pub fn slice_unit(memory_total: u64) -> String {
    let reserve = (memory_total / 10).max(GIB).min(memory_total / 4);
    let memory_max = memory_total - reserve;
    format!(
        "[Unit]\nDescription=Wonton plugins\n\n[Slice]\nCPUWeight=50\nIOWeight=50\nMemoryMax={memory_max}\nTasksMax=50%\n"
    )
}

/// 读取 `/proc/meminfo` 并生成插件 slice；供命令行入口调用。
pub fn slice_unit_from_meminfo(text: &str) -> Result<String, String> {
    Ok(slice_unit(parse_mem_total(text)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_leaves_at_least_one_gib_but_no_more_than_a_quarter() {
        let unit = slice_unit(8 * GIB);
        assert!(unit.contains("MemoryMax=7516192768\n"));
        assert!(!unit.contains("MemoryHigh"));

        let unit = slice_unit(32 * GIB);
        assert!(unit.contains("MemoryMax=30923764532\n"));
    }

    #[test]
    fn mem_total_requires_kib() {
        assert_eq!(parse_mem_total("MemTotal: 1024 kB\n"), Ok(1024 * 1024));
        assert!(parse_mem_total("MemAvailable: 1 kB\n").is_err());
        assert!(parse_mem_total("MemTotal: 1 MB\n").is_err());
    }
}
