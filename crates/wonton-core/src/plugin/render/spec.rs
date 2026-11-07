//! 单个服务的容器规格：compose 服务的字段写进 Docker 的创建和 `HostConfig`。
//!
//! 从 [`super::render_plan`] 的每个服务循环里调用；文字按请求语言拼好整句。

use std::collections::HashMap;

use bollard::models::{
    ContainerCreateBody, EndpointSettings, HealthConfig, HostConfig, Mount, NetworkingConfig,
    RestartPolicy, RestartPolicyNameEnum,
};

use super::super::compose::{
    Healthcheck, NetworkMode, Service, StringOrList, parse_bytes, parse_duration,
};
use super::super::gpu::GpuGrant;
use super::super::interpolate::UsedVars;
use super::super::languages::Languages;
use super::super::requirements::Requirements;
use super::list;
use super::{ContainerPlan, InstallContext, LABEL_SERVICE, RenderError, service_err};

pub(super) struct ContainerOptions<'a> {
    pub requirements: Option<&'a Requirements>,
    pub network: &'a str,
    pub aliases: Vec<String>,
    pub labels: &'a HashMap<String, String>,
    pub gpu: Option<&'a GpuGrant>,
}

/// `endpoint` 是插件网络和服务在其中的别名；`gpu` 是这个服务拿到的显卡（清单的 `gpu.services` 里的服务才有）。
pub(super) fn container_plan(
    name: &str,
    service: &Service,
    mounts: Vec<Mount>,
    ctx: &InstallContext,
    options: ContainerOptions<'_>,
    langs: &Languages,
) -> Result<ContainerPlan, RenderError> {
    let env = match &service.environment {
        Some(kv) => kv
            .pairs("environment", langs)
            .map_err(|m| service_err(langs, name, m))?,
        None => Vec::new(),
    };
    let mut all_labels = match &service.labels {
        Some(kv) => kv
            .pairs("labels", langs)
            .map_err(|m| service_err(langs, name, m))?
            .into_iter()
            .collect(),
        None => HashMap::new(),
    };
    if all_labels.keys().any(|k| k.starts_with("wonton.")) {
        return Err(service_err(
            langs,
            name,
            crate::l10n::text(langs, "plugin-install-render-reserved-label", &[]),
        ));
    }
    all_labels.extend(options.labels.clone());
    all_labels.insert(LABEL_SERVICE.to_owned(), name.to_owned());

    let depends_on = service
        .dependencies()
        .into_iter()
        .map(|(dep, cond)| (ctx.instance.container(&ctx.slug, &dep), cond))
        .collect();
    let networking_config = service.network_mode.is_none().then(|| NetworkingConfig {
        endpoints_config: Some(HashMap::from([(
            options.network.to_owned(),
            EndpointSettings {
                aliases: Some(options.aliases),
                ..Default::default()
            },
        )])),
    });
    let body = ContainerCreateBody {
        image: Some(service.image.clone()),
        cmd: words(name, service.command.as_ref(), langs)?,
        entrypoint: words(name, service.entrypoint.as_ref(), langs)?,
        env: Some(env.into_iter().map(|(k, v)| format!("{k}={v}")).collect()),
        user: service.user.as_ref().map(|u| u.as_string()),
        working_dir: service.working_dir.clone(),
        hostname: service.hostname.clone(),
        labels: Some(all_labels),
        healthcheck: service
            .healthcheck
            .as_ref()
            .map(|h| health_config(name, h, langs))
            .transpose()?,
        stop_signal: service.stop_signal.clone(),
        stop_timeout: service
            .stop_grace_period
            .as_deref()
            .map(|d| parse_duration(d, langs).map(|d| d.as_secs() as i64))
            .transpose()
            .map_err(|m| service_err(langs, name, m))?,
        host_config: Some(host_config(
            name,
            service,
            mounts,
            ctx,
            options.requirements,
            options.network,
            options.gpu,
            langs,
        )?),
        networking_config,
        ..Default::default()
    };
    Ok(ContainerPlan {
        service: name.to_owned(),
        name: ctx.instance.container(&ctx.slug, name),
        image: service.image.clone(),
        pull_policy: service.pull_policy,
        depends_on,
        body,
        files: Vec::new(),
        store_dir: None,
        one_shot: false,
        variables: UsedVars::new(),
    })
}

#[allow(clippy::too_many_arguments)] // 容器 HostConfig 的各部分来自不同来源，再包一层也是逐个传。
fn host_config(
    name: &str,
    service: &Service,
    mounts: Vec<Mount>,
    ctx: &InstallContext,
    requirements: Option<&Requirements>,
    network: &str,
    gpu: Option<&GpuGrant>,
    langs: &Languages,
) -> Result<HostConfig, RenderError> {
    let tmpfs = service.tmpfs.as_ref().map(|spec| {
        list(spec)
            .into_iter()
            .map(|entry| match entry.split_once(':') {
                Some((path, opts)) => (path.to_owned(), opts.to_owned()),
                None => (entry, String::new()),
            })
            .collect()
    });
    let network_mode = match &service.network_mode {
        Some(NetworkMode::None) => "none".to_owned(),
        Some(NetworkMode::Host) => "host".to_owned(),
        Some(NetworkMode::Service(target)) => {
            format!("container:{}", ctx.instance.container(&ctx.slug, target))
        }
        None => network.to_owned(),
    };
    let mut host = HostConfig {
        network_mode: Some(network_mode),
        mounts: Some(mounts),
        restart_policy: Some(restart_policy(name, service.restart.as_deref(), langs)?),
        init: service.init,
        readonly_rootfs: service.read_only,
        shm_size: service
            .shm_size
            .as_ref()
            .map(|v| parse_bytes(v, langs))
            .transpose()
            .map_err(|m| service_err(langs, name, m))?,
        tmpfs,
        extra_hosts: (service.network_mode.is_none() && !ctx.extra_hosts.is_empty())
            .then(|| ctx.extra_hosts.clone()),
        ..Default::default()
    };
    super::super::limits::apply(
        &mut host,
        service,
        requirements,
        ctx.memory_total,
        ctx.cgroup_parent.as_deref(),
        &ctx.limits,
        langs,
    )
    .map_err(|message| service_err(langs, name, message))?;
    if let Some(grant) = gpu {
        grant.apply(&mut host);
    }
    super::super::privileges::apply(service, &mut host, langs)
        .map_err(|message| service_err(langs, name, message))?;
    Ok(host)
}

/// `command` / `entrypoint`：字符串按 shell 规则切分（与 compose 一致），列表原样使用。
fn words(
    service: &str,
    spec: Option<&StringOrList>,
    langs: &Languages,
) -> Result<Option<Vec<String>>, RenderError> {
    match spec {
        None => Ok(None),
        Some(StringOrList::Many(items)) => Ok(Some(items.clone())),
        Some(StringOrList::One(text)) => shlex::split(text).map(Some).ok_or_else(|| {
            service_err(
                langs,
                service,
                crate::l10n::text(
                    langs,
                    "plugin-install-render-quotes-unbalanced",
                    &[("text", text.clone().into())],
                ),
            )
        }),
    }
}

/// 未写 `restart` 时默认 `unless-stopped`：插件应在宿主重启后自动恢复。
fn restart_policy(
    service: &str,
    value: Option<&str>,
    langs: &Languages,
) -> Result<RestartPolicy, RenderError> {
    let policy = |name, retries| RestartPolicy {
        name: Some(name),
        maximum_retry_count: retries,
    };
    Ok(match value.unwrap_or("unless-stopped") {
        "no" => policy(RestartPolicyNameEnum::NO, None),
        "always" => policy(RestartPolicyNameEnum::ALWAYS, None),
        "unless-stopped" => policy(RestartPolicyNameEnum::UNLESS_STOPPED, None),
        "on-failure" => policy(RestartPolicyNameEnum::ON_FAILURE, None),
        other => {
            let retries = other
                .strip_prefix("on-failure:")
                .and_then(|n| n.parse::<i64>().ok())
                .ok_or_else(|| {
                    service_err(
                        langs,
                        service,
                        crate::l10n::text(
                            langs,
                            "plugin-install-render-restart-invalid",
                            &[("value", other.into())],
                        ),
                    )
                })?;
            policy(RestartPolicyNameEnum::ON_FAILURE, Some(retries))
        }
    })
}

fn health_config(
    service: &str,
    check: &Healthcheck,
    langs: &Languages,
) -> Result<HealthConfig, RenderError> {
    let nanos = |value: &Option<String>| -> Result<Option<i64>, RenderError> {
        value
            .as_deref()
            .map(|d| parse_duration(d, langs).map(|d| d.as_nanos() as i64))
            .transpose()
            .map_err(|m| service_err(langs, service, m))
    };
    let test = if check.disable {
        Some(vec!["NONE".to_owned()])
    } else {
        match &check.test {
            None => None,
            Some(StringOrList::One(cmd)) => Some(vec!["CMD-SHELL".to_owned(), cmd.clone()]),
            Some(StringOrList::Many(items)) => {
                let kind = items.first().map(String::as_str);
                if !matches!(kind, Some("CMD" | "CMD-SHELL" | "NONE")) {
                    return Err(service_err(
                        langs,
                        service,
                        crate::l10n::text(langs, "plugin-install-render-healthcheck-format", &[]),
                    ));
                }
                Some(items.clone())
            }
        }
    };
    Ok(HealthConfig {
        test,
        interval: nanos(&check.interval)?,
        timeout: nanos(&check.timeout)?,
        retries: check.retries.map(i64::from),
        start_period: nanos(&check.start_period)?,
        start_interval: nanos(&check.start_interval)?,
    })
}
