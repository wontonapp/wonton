//! 单个服务的容器、中继和挂载：服务字段变成 `ContainerPlan`。
//!
//! 只描述要交给 Docker 的东西；真正创建容器在 [`super::render_plan`] 的调用方。

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use bollard::models::{Mount, MountType};

use super::super::compose::{NetworkMode, Service};
use super::super::configs;
use super::super::egress;
use super::super::gpu::GpuGrant;
use super::super::hostdir::Owner;
use super::super::interpolate::UsedVars;
use super::super::lan;
use super::super::languages::Languages;
use super::super::manifest::{Access, Manifest, Permission};
use super::super::mounts::Mounts;
use super::super::package::PluginPackage;
use super::bot_services::ServiceVars;
use super::host_relay::HostRelay;
use super::spec::{ContainerOptions, container_plan};
use super::{ContainerPlan, InstallContext, RenderError, service_err};

/// 一个服务的容器：Bot 服务每个 Bot 一份，全插件一份的服务只一份。
#[allow(clippy::too_many_arguments)]
pub(super) fn render_service_containers(
    package: &PluginPackage,
    ctx: &InstallContext,
    vars: &ServiceVars,
    service_keys: &[String],
    name: &str,
    service: &Service,
    one_shot: bool,
    gpu: Option<&GpuGrant>,
    requirements: Option<&super::requirements::Requirements>,
    network: &str,
    labels: &HashMap<String, String>,
    read_only_shares: &[PathBuf],
    binds: &mut Vec<(String, PathBuf)>,
    variables: &mut UsedVars,
    containers: &mut Vec<ContainerPlan>,
    langs: &Languages,
) -> Result<(), RenderError> {
    let manifest = &package.manifest;
    // Bot 服务每个 Bot 一份容器；全插件一份的服务只一份。
    let bots: Box<dyn Iterator<Item = Option<&str>>> = if vars.is_bot_service(name) {
        Box::new(vars.bot_names().map(Some))
    } else {
        Box::new(std::iter::once(None))
    };
    for bot in bots {
        let service_vars = match bot {
            Some(bot) => vars.get_bot(bot).ok_or_else(|| {
                service_err(
                    langs,
                    name,
                    format!("没有 Bot `{bot}` 的变量，装不了它的服务"),
                )
            })?,
            None => vars.get(name),
        };
        let scoped_service = bot
            .map(|_| package.bot_service(name, service_vars, variables, langs))
            .transpose()?;
        let service = scoped_service.as_ref().unwrap_or(service);
        // 归属人“AI 动手”是只读的私人 Bot：共享文件夹一律只读挂载（ADR-0088 第 5 节），
        // 插件自己声明了写权限的也一样。
        let forced_shares_read_only = bot
            .and_then(|bot| ctx.bots.get(bot))
            .is_some_and(|vars| vars.keeper_read_only);
        let mut mounts = Mounts {
            ctx,
            read_only_shares,
            forced_shares_read_only,
            named: package.named_volumes(),
            service: name,
            bot,
            allow_docker_socket: ctx.granted.contains(&Permission::DockerSocket),
            socket_declared: manifest.permissions.contains(&Permission::DockerSocket)
                || manifest
                    .optional_permissions
                    .contains_key(&Permission::DockerSocket),
            langs,
        }
        .build(&service.volumes, binds)?;
        mounts.extend(hosted_mounts(manifest, ctx, name));
        super::private_homes::mount_home(manifest, ctx, bot, &mut mounts, service, name, langs)?;
        let (files, store_dir) = configs::resolve_service(
            package,
            service,
            ctx,
            name,
            service_vars,
            &mut mounts,
            variables,
            langs,
        )?;
        // 共用别的服务的网络命名空间时没有自己的网络端点，不设别名（#511）。
        let options = ContainerOptions {
            requirements,
            network,
            aliases: if service.network_mode.is_none() {
                vars.aliases(name, bot)
            } else {
                Vec::new()
            },
            labels,
            gpu,
            bot_services: vars.bot_service_names(),
            manifest,
            devices: ctx.devices.as_ref(),
        };
        let mut plan = container_plan(name, service, bot, mounts, ctx, options, langs)?;
        // 拿到显卡的服务总有这个变量（ADR-0099），插件不必为了它自己的判断再写一行占位环境。
        if let Some(grant) = gpu {
            let env = plan.body.env.get_or_insert_with(Vec::new);
            let prefix = "WONTON_GPU_VENDOR=";
            if !env.iter().any(|value| value.starts_with(prefix)) {
                env.push(format!("{prefix}{}", grant.vendor.key()));
            }
        }
        if let Some(grant) = &ctx.lan {
            lan::publish(name, service, grant, &mut plan.body, langs)?;
        }
        inject_service_egress(ctx, service, service_keys, &mut plan);
        plan.files = files;
        plan.store_dir = store_dir;
        plan.one_shot = one_shot;
        plan.variables = variables.clone();
        containers.push(plan);
    }
    Ok(())
}

fn inject_service_egress(
    ctx: &InstallContext,
    service: &Service,
    service_keys: &[String],
    plan: &mut ContainerPlan,
) {
    // Host-network services cannot resolve the plugin network's proxy alias.
    if let Some(egress) = ctx.egress.as_ref() {
        let url = match &service.network_mode {
            None | Some(NetworkMode::Service(_)) => Some(egress::PROXY_URL),
            Some(NetworkMode::Host) => egress.host_proxy.as_deref(),
            Some(NetworkMode::None) => None,
        };
        if let Some(url) = url {
            egress.inject(&mut plan.body, service_keys.iter().map(String::as_str), url);
            egress::healthcheck::apply(&mut plan.body);
        }
    }
}

/// 中继容器：宿主网络的服务各要一个。核心一侧的套接字路径放不下时在这报出来——
/// 这之后才是拉镜像、起容器，装之前就失败。
pub(super) fn prepare_relays(
    manifest: &Manifest,
    ctx: &InstallContext,
    langs: &Languages,
    services: &BTreeMap<String, Service>,
    containers: &mut Vec<ContainerPlan>,
) -> Result<Vec<HostRelay>, RenderError> {
    let relays = super::host_relay::needed(manifest, services);
    if let Some(problem) =
        super::host_relay::path_too_long(&ctx.relay_dir, &ctx.instance, &ctx.slug, &relays, langs)
    {
        return Err(RenderError::HostPath(problem));
    }
    add_relays(manifest, ctx, &relays, containers);
    Ok(relays)
}

fn add_relays(
    manifest: &Manifest,
    ctx: &InstallContext,
    relays: &[HostRelay],
    containers: &mut Vec<ContainerPlan>,
) {
    let owner = Owner {
        uid: ctx.puid,
        gid: ctx.pgid,
    };
    // 中继目录不在应用数据目录下，`dirs` 里放不进去（见 [`hostdir::prepare`]），安装时单独准备。
    for relay in relays {
        containers.push(super::host_relay::plan(
            &ctx.instance,
            manifest,
            &ctx.relay_dir,
            owner,
            relay,
        ));
    }
}

/// 托管能力的提供方（ADR-0080）：核心把这项能力的总目录挂到它在 `provides.<能力>.mount` 声明的位置，
/// 只读还是读写按能力表。以后装、卸托管插件只改目录里的内容，不重建这个容器。
fn hosted_mounts(manifest: &Manifest, ctx: &InstallContext, service: &str) -> Vec<Mount> {
    let Some(provides) = &manifest.provides else {
        return Vec::new();
    };
    provides
        .capabilities
        .iter()
        .filter(|(_, spec)| spec.service == service)
        .filter_map(|(name, spec)| {
            let hosted = super::capabilities::lookup(name)?.hosted?;
            let read_only = hosted.mount? == super::capabilities::HostedMount::ReadOnly;
            Some(Mount {
                target: Some(spec.mount.clone()?),
                source: Some(ctx.hosted_root.join(name).display().to_string()),
                typ: Some(MountType::BIND),
                read_only: Some(read_only),
                ..Default::default()
            })
        })
        .collect()
}

/// `network_mode: none` 的服务没有网络，网页入口和 MCP 入口不能指向它。
pub(super) fn check_network_mode(
    manifest: &Manifest,
    name: &str,
    service: &Service,
    langs: &Languages,
) -> Result<(), RenderError> {
    if service.network_mode != Some(NetworkMode::None) {
        return Ok(());
    }
    let entries = [
        ("webui", manifest.webui.as_ref().map(|w| &w.service)),
        ("mcp", manifest.mcp.as_ref().map(|m| &m.service)),
    ];
    match entries
        .iter()
        .find(|(_, service)| service.is_some_and(|s| s == name))
    {
        Some((field, _)) => Err(service_err(
            langs,
            name,
            crate::l10n::text(
                langs,
                "plugin-install-render-network-mode-none",
                &[("field", (*field).into())],
            ),
        )),
        None => Ok(()),
    }
}

/// 已授权共享文件夹里服务不可写的那些路径。
pub(super) fn read_only_share_paths(
    manifest: &Manifest,
    shares: &BTreeMap<String, PathBuf>,
) -> Vec<PathBuf> {
    shares
        .iter()
        .filter(|(name, _)| !share_writable(manifest, name))
        .map(|(_, path)| path.clone())
        .collect()
}

fn share_writable(manifest: &Manifest, share: &str) -> bool {
    manifest.permissions.iter().any(
        |p| matches!(p, Permission::StorageShare { name, access: Access::Write } if name == share),
    )
}

pub(super) fn dependencies(services: &BTreeMap<String, Service>) -> BTreeMap<&str, Vec<String>> {
    services
        .iter()
        .map(|(name, service)| {
            let deps = service
                .dependencies()
                .into_iter()
                .map(|(name, _)| name)
                .collect();
            (name.as_str(), deps)
        })
        .collect()
}
