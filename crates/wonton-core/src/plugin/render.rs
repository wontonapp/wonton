//! 渲染：把插件包和安装环境变成要交给 Docker 的部署计划。
//!
//! 这里是纯函数，不访问 Docker。注入的变量见 `docs/plugin-spec.md` 的“安装时注入的变量”。

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use bollard::models::{
    ContainerCreateBody, EndpointSettings, HealthConfig, HostConfig, Mount, NetworkingConfig,
    RestartPolicy, RestartPolicyNameEnum,
};

use super::bot_services::ServiceVars;
use super::compose::{
    DependCondition, Healthcheck, NetworkMode, PullPolicy, Service, StringOrList, parse_bytes,
    parse_duration,
};
use super::configs::{self, ConfigError, ConfigFile};
use super::core_address;
use super::egress::{self, EgressVars};
use super::gpu::GpuGrant;
use super::host_relay::HostRelay;
use super::hostdir::Owner;
use super::lan::{self, LanGrant};
use super::limits;
use super::manifest::{Access, Manifest, Permission, SecretSpec, Sso};
use super::mounts::{Mounts, check_not_nested, dir_owners, mount_targets};
use super::names::Instance;
use super::package::{PackageError, PluginPackage};
use super::requirements::Requirements;
use super::vars::injected_vars;

/// 容器是插件的哪个服务。
pub const LABEL_SERVICE: &str = "wonton.service";

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error(transparent)]
    Package(#[from] PackageError),
    #[error("服务 `{service}`：{message}")]
    Service { service: String, message: String },
    #[error("插件没申请 owner.identity，不能引用 `{0}`：在清单的 permissions 里加上它")]
    NoOwnerIdentity(String),
    #[error("{}", crate::owner::NO_EMAIL)]
    NoOwnerEmail,
}

/// 引用了主人的身份变量却没注入时，分清是插件没申请还是主人没配（ADR-0022）。
fn explain_owner_vars(error: RenderError, ctx: &InstallContext) -> RenderError {
    let variable = match &error {
        RenderError::Package(
            PackageError::Interpolate { source, .. }
            | PackageError::ConfigInterpolate { source, .. },
        ) => source.variable(),
        _ => None,
    };
    match (variable, &ctx.owner) {
        (Some("WONTON_OWNER_EMAIL"), Some(_)) => RenderError::NoOwnerEmail,
        (Some(name @ ("WONTON_OWNER_NAME" | "WONTON_OWNER_EMAIL")), None) => {
            RenderError::NoOwnerIdentity(name.to_owned())
        }
        _ => error,
    }
}

pub(super) fn service_err(service: &str, message: impl Into<String>) -> RenderError {
    RenderError::Service {
        service: service.to_owned(),
        message: message.into(),
    }
}

/// 注入给 OIDC 客户端的配置。`Debug` 不打印客户端密钥。
#[derive(Clone, PartialEq, Eq)]
pub struct OidcVars {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub userinfo_url: String,
}

impl std::fmt::Debug for OidcVars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcVars")
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

/// 注入给插件的主人身份（[`Manifest::gets_owner_identity`]），和 OIDC 声明、代登录的 `{user}`、`{email}` 同值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerVars {
    pub name: String,
    pub email: Option<String>,
}

/// 注入给 Bot 的 Matrix 身份（`matrix.bot`）。`Debug` 不打印访问令牌。
#[derive(Clone, PartialEq, Eq)]
pub struct MatrixVars {
    pub homeserver: String,
    pub user_id: String,
    pub access_token: String,
    pub owner: String,
    pub home_room: String,
}

impl std::fmt::Debug for MatrixVars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MatrixVars")
            .field("homeserver", &self.homeserver)
            .field("user_id", &self.user_id)
            .field("owner", &self.owner)
            .field("home_room", &self.home_room)
            .finish_non_exhaustive()
    }
}

/// 注入给使用方的模型网关配置（`models`）。`Debug` 不打印凭据。
#[derive(Clone, PartialEq, Eq)]
pub struct ModelVars {
    pub base_url: String,
    pub anthropic_url: Option<String>,
    pub gemini_url: Option<String>,
    /// 插件级的凭据（凭据名是插件 `id`），全插件一份的服务用。
    pub plugin_key: Option<String>,
    /// 默认 Bot 的凭据（`<id>/default`），声明了 `bots` 时 Bot 服务用。
    pub bot_key: Option<String>,
    pub default_model: String,
}

impl std::fmt::Debug for ModelVars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelVars")
            .field("base_url", &self.base_url)
            .field("anthropic_url", &self.anthropic_url)
            .field("gemini_url", &self.gemini_url)
            .field("default_model", &self.default_model)
            .finish_non_exhaustive()
    }
}

/// 安装环境：由核心配置和本次安装分配的资源决定。`Debug` 不打印其中的令牌和密钥。
#[derive(Clone)]
pub struct InstallContext {
    pub slug: String,
    /// 资源名和标签带不带实例名。
    pub instance: Instance,
    /// 插件私有数据目录（宿主路径）。
    pub app_data: PathBuf,
    /// 已授权的共享文件夹：名称 → 宿主路径。
    pub shares: BTreeMap<String, PathBuf>,
    /// 插件网页的公开地址（不带末尾斜杠）。
    pub app_url: Option<String>,
    pub oidc: Option<OidcVars>,
    /// 声明了 `webui.sso`（`oidc` 或 `proxy`）时：主人的用户名和邮箱。这些插件本来就能从登录里得到它们，其余的
    /// 插件（包括未审核的）不多拿一份主人的个人信息。
    pub owner: Option<OwnerVars>,
    /// 声明了 `matrix.bot` 时：核心代 Bot 登录后签发的身份。
    pub matrix: Option<MatrixVars>,
    /// 声明了 `models` 时：模型网关的地址、凭据和默认模型。
    pub models: Option<ModelVars>,
    /// 声明了 `tools` 时：MCP 总线的地址。
    pub mcp_url: Option<String>,
    /// 核心生成的插件密钥（`secrets` 里的 `generate` 项）：名字 → 值。
    pub secrets: BTreeMap<String, String>,
    pub tz: String,
    pub puid: u32,
    pub pgid: u32,
    /// 额外写进每个容器 `/etc/hosts` 的条目（核心不在容器里运行时，用来让插件访问网关）。
    pub extra_hosts: Vec<String>,
    /// 申请了 `devices.gpu` 且本机有可用的显卡时：交给 `gpu.services` 里的服务的设备（见 [`super::gpu`]）。
    pub gpu: Option<GpuGrant>,
    /// 申请了 `network.lan` 时：局域网端口开在哪里（见 [`super::lan`]）。
    pub lan: Option<LanGrant>,
    /// 声明了 `network.egress` 且主人配了代理时：要注入的代理变量（见 [`super::egress`]）。
    pub egress: Option<EgressVars>,
    /// 声明了 `webui.sso: proxy` 时：核心在这个插件网络里的地址（见 [`super::core_address`]）。
    pub core_address: Option<String>,
    /// 宿主总内存；未知时插件服务不设置默认内存上限。
    pub memory_total: Option<u64>,
    /// 主人给插件服务设置的覆盖上限；安装和 dry run 使用默认值。
    pub limits: wonton_protocol::PluginLimits,
    /// 插件 systemd slice；没有安装 slice 时不设置父 cgroup。
    pub cgroup_parent: Option<String>,
}

impl std::fmt::Debug for InstallContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallContext")
            .field("slug", &self.slug)
            .field("instance", &self.instance)
            .field("app_data", &self.app_data)
            .field("shares", &self.shares)
            .field("app_url", &self.app_url)
            .field("oidc", &self.oidc)
            .field("owner", &self.owner)
            .field("matrix", &self.matrix)
            .field("models", &self.models)
            .field("mcp_url", &self.mcp_url)
            .field("secrets", &self.secrets.keys().collect::<Vec<_>>())
            .field("tz", &self.tz)
            .field("puid", &self.puid)
            .field("pgid", &self.pgid)
            .field("extra_hosts", &self.extra_hosts)
            .field("gpu", &self.gpu)
            .field("lan", &self.lan)
            .field("egress", &self.egress)
            .field("core_address", &self.core_address)
            .field("memory_total", &self.memory_total)
            .field("limits", &self.limits)
            .field("cgroup_parent", &self.cgroup_parent)
            .finish()
    }
}
#[derive(Debug, Clone)]
pub struct ContainerPlan {
    pub service: String,
    pub name: String,
    pub image: String,
    pub pull_policy: PullPolicy,
    /// 启动前要等待的容器及条件。
    pub depends_on: Vec<(String, DependCondition)>,
    pub body: ContainerCreateBody,
    /// 创建之后、启动之前写进容器的配置文件。
    pub files: Vec<ConfigFile>,
    /// 一次性服务：以 0 退出是它的正常状态，安装时不等它健康。
    pub one_shot: bool,
}

struct ContainerOptions<'a> {
    requirements: Option<&'a Requirements>,
    network: &'a str,
    aliases: Vec<String>,
    labels: &'a HashMap<String, String>,
    gpu: Option<&'a GpuGrant>,
}

#[derive(Debug, Clone)]
pub struct Deployment {
    pub network: String,
    pub labels: HashMap<String, String>,
    /// 要创建的 Docker 命名卷。
    pub volumes: Vec<String>,
    /// 要预先创建的宿主目录（应用数据目录下的绑定挂载源）和各自的属主。
    pub dirs: BTreeMap<PathBuf, Owner>,
    /// 按依赖排好序的容器。
    pub containers: Vec<ContainerPlan>,
    /// 宿主网络网页和 MCP 服务的中继。
    pub relays: Vec<HostRelay>,
}

/// 用占位环境渲染一遍，在不接触 Docker 的情况下检查插件包能否安装。
pub fn dry_run(package: &PluginPackage) -> Result<Deployment, RenderError> {
    dry_run_with_email(package, Some("owner@wonton.invalid"))
}

/// 同 [`dry_run`]，主人的邮箱用 `email`：主人没配邮箱时，看插件在这台主机上装不装得上。
pub(super) fn dry_run_with_email(
    package: &PluginPackage,
    email: Option<&str>,
) -> Result<Deployment, RenderError> {
    render(package, &dry_run_context(package, email))
}

/// [`dry_run`] 用的占位环境。
pub(super) fn dry_run_context(package: &PluginPackage, email: Option<&str>) -> InstallContext {
    let manifest = &package.manifest;
    let slug = manifest.id.slug();
    let root = PathBuf::from("/wonton-dry-run");
    let shares = manifest
        .permissions
        .iter()
        .filter_map(|p| match p {
            Permission::StorageShare { name, .. } => {
                Some((name.clone(), root.join("shares").join(name)))
            }
            _ => None,
        })
        .collect();
    let webui = manifest.webui.as_ref();
    let url = format!("http://{slug}.wonton.invalid");
    InstallContext {
        app_data: root.join("apps").join(&slug),
        shares,
        app_url: webui.map(|_| url.clone()),
        oidc: webui.filter(|w| w.sso == Sso::Oidc).map(|_| OidcVars {
            issuer: "http://auth.wonton.invalid".into(),
            client_id: manifest.id.to_string(),
            client_secret: "dry-run".into(),
            auth_url: "http://auth.wonton.invalid/auth".into(),
            token_url: "http://auth.wonton.invalid/token".into(),
            userinfo_url: "http://auth.wonton.invalid/userinfo".into(),
        }),
        owner: manifest.gets_owner_identity().then(|| OwnerVars {
            name: "owner".into(),
            email: email.map(str::to_owned),
        }),
        matrix: manifest
            .permissions
            .contains(&Permission::MatrixBot)
            .then(|| MatrixVars {
                homeserver: "http://matrix.wonton.internal".into(),
                user_id: "@dry-run:wonton.invalid".into(),
                access_token: "dry-run".into(),
                owner: "@owner:wonton.invalid".into(),
                home_room: "!dry-run:wonton.invalid".into(),
            }),
        models: manifest
            .permissions
            .contains(&Permission::Models)
            .then(|| ModelVars {
                base_url: "http://models.wonton.internal/v1".into(),
                anthropic_url: Some("http://models.wonton.internal".into()),
                gemini_url: Some("http://models.wonton.internal".into()),
                plugin_key: Some("dry-run".into()),
                bot_key: manifest.bots.as_ref().map(|_| "dry-run".into()),
                default_model: "dry-run".into(),
            }),
        mcp_url: manifest
            .permissions
            .contains(&Permission::Tools)
            .then(|| "http://mcp.wonton.internal/mcp".into()),
        secrets: manifest
            .secrets
            .iter()
            .filter(|(_, spec)| matches!(spec, SecretSpec::Generate(_)))
            .map(|(name, _)| (name.clone(), "dry-run".to_owned()))
            .collect(),
        instance: Instance::default(),
        tz: "UTC".into(),
        puid: 1000,
        pgid: 1000,
        extra_hosts: Vec::new(),
        gpu: None,
        // 192.0.2.1 是文档用的地址（RFC 5737）。
        lan: manifest.lan.as_ref().map(|lan| LanGrant {
            addresses: vec![[192, 0, 2, 1].into()],
            network: (!manifest.permissions.contains(&Permission::NetworkEgress))
                .then(|| Instance::default().lan_network(&slug)),
            ports: lan.ports.clone(),
            webui: false,
        }),
        egress: None,
        core_address: webui
            .filter(|w| w.sso == Sso::Proxy)
            .map(|_| core_address::PLACEHOLDER.to_owned()),
        limits: Default::default(),
        memory_total: None,
        cgroup_parent: None,
        slug,
    }
}

pub fn render(package: &PluginPackage, ctx: &InstallContext) -> Result<Deployment, RenderError> {
    render_plan(package, ctx).map_err(|e| explain_owner_vars(e, ctx))
}

fn render_plan(package: &PluginPackage, ctx: &InstallContext) -> Result<Deployment, RenderError> {
    let manifest = &package.manifest;
    let requirements = manifest.requirements.as_ref();
    let bot_key = ctx.models.as_ref().and_then(|m| m.bot_key.clone());
    let vars = ServiceVars::new(
        manifest,
        &ctx.app_data,
        injected_vars(manifest, ctx),
        bot_key,
    );
    let services = package.services(|name| vars.get(name))?;
    if let Err(error) = super::privileges::validate_services(manifest, &services) {
        return Err(service_err(&error.service, error.message));
    }
    if let Some((service, bot_service)) = vars.plugin_service_on_bot(&services) {
        return Err(service_err(
            &service,
            format!("全插件一份的服务不能依赖 Bot 服务 `{bot_service}`：Bot 服务每个 Bot 一份"),
        ));
    }
    let network = ctx.instance.network(&ctx.slug);
    let labels = ctx.instance.labels(&manifest.id);
    let read_only_shares: Vec<PathBuf> = ctx
        .shares
        .iter()
        .filter(|(name, _)| !share_writable(manifest, name))
        .map(|(_, path)| path.clone())
        .collect();
    let one_shot = package.one_shot_services();
    let gpu_services: &[String] = manifest.gpu.as_ref().map_or(&[], |g| &g.services);
    let mut binds = Vec::new();
    let mut containers = Vec::new();
    for name in start_order(dependencies(&services))? {
        let service = &services[&name];
        check_network_mode(manifest, &name, service)?;
        let gpu = ctx.gpu.as_ref().filter(|_| gpu_services.contains(&name));
        let one_shot = one_shot.contains(name.as_str());
        if one_shot && service.restart.as_deref() != Some("no") {
            return Err(service_err(
                &name,
                "被以 service_completed_successfully 依赖的一次性服务必须写 restart: \"no\"：\
                 否则 Docker 会在它退出后反复重启它",
            ));
        }
        let mounts = Mounts {
            ctx,
            read_only_shares: &read_only_shares,
            named: package.named_volumes(),
            service: &name,
            allow_docker_socket: manifest.permissions.contains(&Permission::DockerSocket),
        }
        .build(&service.volumes, &mut binds)?;
        let targets = mount_targets(service, &mounts);
        let files =
            configs::resolve(package, service, vars.get(&name), &targets).map_err(|e| match e {
                ConfigError::Package(e) => e.into(),
                ConfigError::Rule(m) => service_err(&name, m),
            })?;
        let options = ContainerOptions {
            requirements,
            network: network.as_str(),
            aliases: vars.aliases(&name),
            labels: &labels,
            gpu,
        };
        let mut plan = container_plan(&name, service, mounts, ctx, options)?;
        if let Some(grant) = &ctx.lan {
            lan::publish(&name, service, grant, &mut plan.body)?;
        }
        if let Some(vars) = ctx.egress.as_ref()
            && service.network_mode.is_none()
        {
            vars.inject(&mut plan.body, services.keys().map(String::as_str));
            egress::healthcheck::apply(&mut plan.body);
        }
        plan.files = files;
        plan.one_shot = one_shot;
        containers.push(plan);
    }
    check_not_nested(&binds)?;
    let mut dirs = dir_owners(&binds, &services, ctx)?;
    let relays = super::host_relay::needed(manifest, &services);
    add_relays(manifest, ctx, &relays, &mut containers, &mut dirs);
    let volumes = package
        .named_volumes()
        .iter()
        .map(|v| ctx.instance.volume(&ctx.slug, v))
        .collect();
    Ok(Deployment {
        network,
        labels,
        volumes,
        dirs,
        containers,
        relays,
    })
}

fn add_relays(
    manifest: &Manifest,
    ctx: &InstallContext,
    relays: &[HostRelay],
    containers: &mut Vec<ContainerPlan>,
    dirs: &mut BTreeMap<PathBuf, Owner>,
) {
    let owner = Owner {
        uid: ctx.puid,
        gid: ctx.pgid,
    };
    for relay in relays {
        dirs.insert(super::host_relay::dir(&ctx.app_data), owner);
        containers.push(super::host_relay::plan(
            &ctx.instance,
            manifest,
            &ctx.app_data,
            owner,
            relay,
        ));
    }
}

/// `network_mode: none` 的服务没有网络，网页入口和 MCP 入口不能指向它。
fn check_network_mode(
    manifest: &Manifest,
    name: &str,
    service: &Service,
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
            name,
            format!("{field} 指向的服务不能写 network_mode: none"),
        )),
        None => Ok(()),
    }
}

fn share_writable(manifest: &Manifest, share: &str) -> bool {
    manifest.permissions.iter().any(
        |p| matches!(p, Permission::StorageShare { name, access: Access::Write } if name == share),
    )
}

fn dependencies(services: &BTreeMap<String, Service>) -> BTreeMap<&str, Vec<String>> {
    services
        .iter()
        .map(|(name, service)| {
            let deps = service
                .depends_on
                .as_ref()
                .map(|deps| deps.entries().into_iter().map(|(name, _)| name).collect())
                .unwrap_or_default();
            (name.as_str(), deps)
        })
        .collect()
}

/// 按 `depends_on` 拓扑排序；同层按名字排序，保证结果确定。
pub(super) fn start_order(
    mut pending: BTreeMap<&str, Vec<String>>,
) -> Result<Vec<String>, RenderError> {
    for (name, deps) in &pending {
        if let Some(missing) = deps.iter().find(|d| !pending.contains_key(d.as_str())) {
            return Err(service_err(
                name,
                format!("depends_on 引用了不存在的服务 `{missing}`"),
            ));
        }
    }
    let mut order: Vec<String> = Vec::new();
    while !pending.is_empty() {
        let ready: Vec<&str> = pending
            .iter()
            .filter(|(_, deps)| deps.iter().all(|d| order.contains(d)))
            .map(|(name, _)| *name)
            .collect();
        if ready.is_empty() {
            let names: Vec<&str> = pending.keys().copied().collect();
            return Err(service_err(&names.join(", "), "depends_on 存在循环依赖"));
        }
        for name in ready {
            pending.remove(name);
            order.push(name.to_owned());
        }
    }
    Ok(order)
}

/// `endpoint` 是插件网络和服务在其中的别名；`gpu` 是这个服务拿到的显卡（清单的 `gpu.services` 里的服务才有）。
fn container_plan(
    name: &str,
    service: &Service,
    mounts: Vec<Mount>,
    ctx: &InstallContext,
    options: ContainerOptions<'_>,
) -> Result<ContainerPlan, RenderError> {
    let env = match &service.environment {
        Some(kv) => kv.pairs("environment").map_err(|m| service_err(name, m))?,
        None => Vec::new(),
    };
    let mut all_labels = match &service.labels {
        Some(kv) => kv
            .pairs("labels")
            .map_err(|m| service_err(name, m))?
            .into_iter()
            .collect(),
        None => HashMap::new(),
    };
    if all_labels.keys().any(|k| k.starts_with("wonton.")) {
        return Err(service_err(name, "`wonton.` 开头的标签由核心保留"));
    }
    all_labels.extend(options.labels.clone());
    all_labels.insert(LABEL_SERVICE.to_owned(), name.to_owned());

    let depends_on = service
        .depends_on
        .as_ref()
        .map(|d| d.entries())
        .unwrap_or_default()
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
        cmd: words(name, service.command.as_ref())?,
        entrypoint: words(name, service.entrypoint.as_ref())?,
        env: Some(env.into_iter().map(|(k, v)| format!("{k}={v}")).collect()),
        user: service.user.as_ref().map(|u| u.as_string()),
        working_dir: service.working_dir.clone(),
        hostname: service.hostname.clone(),
        labels: Some(all_labels),
        healthcheck: service
            .healthcheck
            .as_ref()
            .map(|h| health_config(name, h))
            .transpose()?,
        stop_signal: service.stop_signal.clone(),
        stop_timeout: service
            .stop_grace_period
            .as_deref()
            .map(|d| parse_duration(d).map(|d| d.as_secs() as i64))
            .transpose()
            .map_err(|m| service_err(name, m))?,
        host_config: Some(host_config(
            name,
            service,
            mounts,
            ctx,
            options.requirements,
            options.network,
            options.gpu,
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
        one_shot: false,
    })
}

fn host_config(
    name: &str,
    service: &Service,
    mounts: Vec<Mount>,
    ctx: &InstallContext,
    requirements: Option<&Requirements>,
    network: &str,
    gpu: Option<&GpuGrant>,
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
    let network_mode = match service.network_mode {
        Some(NetworkMode::None) => "none",
        Some(NetworkMode::Host) => "host",
        None => network,
    };
    let mut host = HostConfig {
        network_mode: Some(network_mode.to_owned()),
        mounts: Some(mounts),
        restart_policy: Some(restart_policy(name, service.restart.as_deref())?),
        init: service.init,
        readonly_rootfs: service.read_only,
        shm_size: service
            .shm_size
            .as_ref()
            .map(parse_bytes)
            .transpose()
            .map_err(|m| service_err(name, m))?,
        tmpfs,
        extra_hosts: (service.network_mode.is_none() && !ctx.extra_hosts.is_empty())
            .then(|| ctx.extra_hosts.clone()),
        ..Default::default()
    };
    limits::apply(
        &mut host,
        service,
        requirements,
        ctx.memory_total,
        ctx.cgroup_parent.as_deref(),
        &ctx.limits,
    )
    .map_err(|message| service_err(name, message))?;
    if let Some(grant) = gpu {
        grant.apply(&mut host);
    }
    super::privileges::apply(service, &mut host).map_err(|message| service_err(name, message))?;
    Ok(host)
}

pub(super) fn list(spec: &StringOrList) -> Vec<String> {
    match spec {
        StringOrList::One(s) => vec![s.clone()],
        StringOrList::Many(items) => items.clone(),
    }
}

/// `command` / `entrypoint`：字符串按 shell 规则切分（与 compose 一致），列表原样使用。
fn words(service: &str, spec: Option<&StringOrList>) -> Result<Option<Vec<String>>, RenderError> {
    match spec {
        None => Ok(None),
        Some(StringOrList::Many(items)) => Ok(Some(items.clone())),
        Some(StringOrList::One(text)) => shlex::split(text)
            .map(Some)
            .ok_or_else(|| service_err(service, format!("命令 `{text}` 的引号不配对"))),
    }
}

/// 未写 `restart` 时默认 `unless-stopped`：插件应在宿主重启后自动恢复。
fn restart_policy(service: &str, value: Option<&str>) -> Result<RestartPolicy, RenderError> {
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
                .ok_or_else(|| service_err(service, format!("restart 取值 `{other}` 不合法")))?;
            policy(RestartPolicyNameEnum::ON_FAILURE, Some(retries))
        }
    })
}

fn health_config(service: &str, check: &Healthcheck) -> Result<HealthConfig, RenderError> {
    let nanos = |value: &Option<String>| -> Result<Option<i64>, RenderError> {
        value
            .as_deref()
            .map(|d| parse_duration(d).map(|d| d.as_nanos() as i64))
            .transpose()
            .map_err(|m| service_err(service, m))
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
                        service,
                        "healthcheck.test 列表须以 CMD、CMD-SHELL 或 NONE 开头",
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
