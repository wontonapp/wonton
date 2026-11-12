//! 渲染：把插件包和安装环境变成要交给 Docker 的部署计划。
//!
//! 这里是纯函数，不访问 Docker。注入的变量见 `docs/plugin-spec.md` 的“安装时注入的变量”。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bollard::models::{ContainerCreateBody, Mount, MountType};

use super::bot_services::{BotVars, ServiceVars};
use super::compose::{DependCondition, NetworkMode, PullPolicy, Service, StringOrList};
use super::configs::{self, ConfigFile};
use super::core_address;
use super::egress::{self, EgressVars};
use super::gpu::GpuGrant;
use super::host_relay::HostRelay;
use super::hostdir::Owner;
use super::interpolate::UsedVars;
use super::lan::{self, LanGrant};
use super::languages::Languages;
use super::manifest::{Access, Manifest, Permission, Sso};
use super::mounts::{Mounts, check_not_nested, dir_owners};
use super::names::Instance;
use super::package::{PackageError, PluginPackage};
use super::vars::injected_vars;
pub use super::vars::{MatrixVars, ModelVars, OidcVars, OwnerVars};
use spec::{ContainerOptions, container_plan};

/// 容器是插件的哪个服务。
pub const LABEL_SERVICE: &str = "wonton.service";
/// 容器是哪个 Bot 的：只有 Bot 服务（`bots.services`）的容器有。
pub const LABEL_BOT: &str = "wonton.bot";

pub(super) mod spec;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error(transparent)]
    Package(#[from] PackageError),
    #[error("{message}")]
    Service { service: String, message: String },
    /// 这台主机的目录放不下插件要的资源（如中继套接字路径超长），不是插件包的问题。
    #[error("{0}")]
    HostPath(String),
    #[error("{0}")]
    NoOwnerIdentity(String),
}

/// 引用了主人的身份变量却没申请身份权限时，说明缺少的声明。
fn explain_owner_vars(error: RenderError, ctx: &InstallContext, langs: &Languages) -> RenderError {
    let variable = match &error {
        RenderError::Package(
            PackageError::Interpolate { source, .. }
            | PackageError::ConfigInterpolate { source, .. },
        ) => source.variable(),
        _ => None,
    };
    match (variable, &ctx.owner) {
        (Some(name @ ("WONTON_OWNER_NAME" | "WONTON_OWNER_EMAIL")), None) => {
            RenderError::NoOwnerIdentity(crate::l10n::text(
                langs,
                "plugin-install-render-no-owner-identity",
                &[("var", name.into())],
            ))
        }
        _ => error,
    }
}

/// `message` 已是按语言拼好的整句；这里补上「服务 {service}：」前缀（plugin-install-service-error）。
pub(super) fn service_err(
    langs: &Languages,
    service: &str,
    message: impl Into<String>,
) -> RenderError {
    let message: String = message.into();
    RenderError::Service {
        service: service.to_owned(),
        message: crate::l10n::text(
            langs,
            "plugin-install-service-error",
            &[("service", service.into()), ("error", message.into())],
        ),
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
    /// 托管形态的总目录（宿主路径，ADR-0080）：提供方挂 `<它>/<能力名>`。
    pub hosted_root: PathBuf,
    /// 核心管理的配置文件目录（宿主路径），`read_only` 服务的 `configs` 从这里挂进容器。
    pub config_store: PathBuf,
    /// 已授权的共享文件夹：名称 → 宿主路径。
    pub shares: BTreeMap<String, PathBuf>,
    /// 插件网页的公开地址（不带末尾斜杠）。
    pub app_url: Option<String>,
    pub oidc: Option<OidcVars>,
    /// 声明了 `webui.sso`（`oidc` 或 `proxy`）时：主人的用户名和邮箱。这些插件本来就能从登录里得到它们，其余的
    /// 插件（包括未审核的）不多拿一份主人的个人信息。
    pub owner: Option<OwnerVars>,
    /// 声明了 `matrix.bot` 时：核心代 Bot 登录后签发的身份。只有一个 Bot（没声明 `bots`）的插件在
    /// 插件级变量里注入它；声明了 `bots` 的按 Bot 注入，见 `bots`。
    pub matrix: Option<MatrixVars>,
    /// 声明了 `bots` 时每个 Bot 的 Matrix 身份、模型凭据和总线地址（`bots.services` 里的服务用）。
    pub bots: BTreeMap<String, BotVars>,
    /// 声明了 `models` 时：模型网关的地址、凭据和默认模型。
    pub models: Option<ModelVars>,
    /// 通用能力的地址和凭据，仅用于清单插值；Debug 不打印值。
    pub capabilities: BTreeMap<String, String>,
    /// 声明了 `tools` 时：MCP 总线的地址。
    pub mcp_url: Option<String>,
    /// 声明了 `notify` 时：发通知的地址（ADR-0047）。
    pub notify_url: Option<String>,
    /// 核心生成的插件密钥（`secrets` 里的 `generate` 项）：名字 → 值。
    pub secrets: BTreeMap<String, String>,
    /// 主人设置的普通值（ADR-0077）：属性名 → JSON 值。
    pub settings: BTreeMap<String, serde_json::Value>,
    pub tz: String,
    pub puid: u32,
    pub pgid: u32,
    /// 这个插件最多等多久：核心的安装期限和清单声明的期限取大的（见
    /// [`PluginManager::install_limit`](super::manager::PluginManager::install_limit)）。安装、启动、重建都按它
    /// 算健康门的时限。
    pub install_limit: Duration,
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
    /// 宿主主机名（`WONTON_HOST_NAME`）；取不到时为空。
    pub host_name: String,
    /// 主人给插件服务设置的覆盖上限；安装和 dry run 使用默认值。
    pub limits: wonton_protocol::PluginLimits,
    /// 插件 systemd slice；没有安装 slice 时不设置父 cgroup。
    pub cgroup_parent: Option<String>,
    /// 生效集：主人确认的必需权限加上开着的可选项（ADR-0057）。渲染按它决定可选字段给不给。
    pub granted: Vec<Permission>,
}

impl std::fmt::Debug for InstallContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallContext")
            .field("slug", &self.slug)
            .field("instance", &self.instance)
            .field("app_data", &self.app_data)
            .field("config_store", &self.config_store)
            .field("shares", &self.shares)
            .field("app_url", &self.app_url)
            .field("oidc", &self.oidc)
            .field("owner", &self.owner)
            .field("matrix", &self.matrix)
            .field("bots", &self.bots.keys().collect::<Vec<_>>())
            .field("models", &self.models)
            .field("mcp_url", &self.mcp_url)
            .field("notify_url", &self.notify_url)
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
            .field("host_name", &self.host_name)
            .field("limits", &self.limits)
            .field("cgroup_parent", &self.cgroup_parent)
            .field("granted", &self.granted)
            .finish()
    }
}
#[derive(Debug, Clone)]
pub struct ContainerPlan {
    pub service: String,
    /// Bot 服务的容器属于哪个 Bot（`bots.services`）；全插件一份的服务是 `None`。
    pub bot: Option<String>,
    pub name: String,
    pub image: String,
    pub pull_policy: PullPolicy,
    /// 启动前要等待的容器及条件。
    pub depends_on: Vec<(String, DependCondition)>,
    pub body: ContainerCreateBody,
    /// 创建之后、启动之前写进容器的配置文件。
    pub files: Vec<ConfigFile>,
    /// `read_only` 服务的配置文件在核心数据目录里的目录（[`super::configs_store`]）：文件已按只读
    /// 绑定挂载配好，`files` 交给一次性容器经归档接口写进这里，而不是写进容器根。
    pub store_dir: Option<PathBuf>,
    /// 一次性服务：以 0 退出是它的正常状态，安装时不等它健康。
    pub one_shot: bool,
    /// 渲染这个容器时用到的变量名：局域网地址变了，引用它的容器要重建。
    pub variables: UsedVars,
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
pub fn dry_run(package: &PluginPackage, langs: &Languages) -> Result<Deployment, RenderError> {
    render(package, &dry_run_context(package), langs)
}

/// [`dry_run`] 用的占位环境。
pub(super) fn dry_run_context(package: &PluginPackage) -> InstallContext {
    let manifest = &package.manifest;
    let slug = manifest.id.slug();
    let root = PathBuf::from("/wonton-dry-run");
    let shares = dry_run_shares(manifest, &root);
    let webui = manifest.webui.as_ref();
    let url = format!("http://{slug}.wonton.invalid");
    // 所有密钥项都占位：`collect` 项也会在容器 env 里被引用，漏给会误报“变量未定义”（#775）。
    let secrets = manifest
        .secrets
        .keys()
        .map(|name| (name.clone(), "dry-run".to_owned()))
        .collect();
    InstallContext {
        app_data: root.join("apps").join(&slug),
        hosted_root: root.join("hosted"),
        config_store: root.join("core").join("configs"),
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
            email: crate::owner::internal_email("owner"),
        }),
        matrix: manifest
            .permissions
            .contains(&Permission::MatrixBot)
            .then(|| MatrixVars {
                homeserver: "http://matrix.wonton.internal".into(),
                user_id: "@dry-run:wonton.invalid".into(),
                access_token: "dry-run".into(),
                allowed_users: "@owner:wonton.invalid".into(),
                home_room: "!dry-run:wonton.invalid".into(),
            }),
        bots: dry_run_bots(manifest),
        models: manifest
            .permissions
            .contains(&Permission::Models)
            .then(|| ModelVars {
                base_url: "http://models.wonton.internal/v1".into(),
                anthropic_url: Some("http://models.wonton.internal".into()),
                gemini_url: Some("http://models.wonton.internal".into()),
                plugin_key: Some("dry-run".into()),
                bot_keys: manifest
                    .bots
                    .as_ref()
                    .map(|_| {
                        BTreeMap::from([(
                            super::bot_services::DEFAULT_BOT.to_owned(),
                            "dry-run".into(),
                        )])
                    })
                    .unwrap_or_default(),
                default_model: "dry-run".into(),
            }),
        capabilities: dry_run_capabilities(manifest, &root.join("apps").join(&slug)),
        mcp_url: manifest
            .permissions
            .contains(&Permission::Tools)
            .then(|| "http://mcp.wonton.internal/mcp".into()),
        notify_url: manifest
            .permissions
            .contains(&Permission::Notify)
            .then(|| "http://notify.wonton.internal/notices".into()),
        secrets,
        settings: super::settings::defaults(package),
        instance: Instance::default(),
        tz: "UTC".into(),
        puid: 1000,
        pgid: 1000,
        install_limit: crate::config::DEFAULT_INSTALL_TIMEOUT,
        extra_hosts: Vec::new(),
        gpu: None,
        lan: dry_run_lan(manifest, &slug),
        egress: None,
        core_address: webui
            .filter(|w| w.sso == Sso::Proxy)
            .map(|_| core_address::PLACEHOLDER.to_owned()),
        limits: Default::default(),
        memory_total: None,
        host_name: String::new(),
        cgroup_parent: None,
        slug,
        granted: manifest.effective_permissions(
            &manifest
                .optional_permissions
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
        ),
    }
}

fn dry_run_lan(manifest: &Manifest, slug: &str) -> Option<LanGrant> {
    // 192.0.2.1 是文档用的地址（RFC 5737）。
    manifest.lan.as_ref().map(|lan| LanGrant {
        addresses: vec![[192, 0, 2, 1].into()],
        announce: vec![[192, 0, 2, 1].into()],
        network: (!manifest.permissions.contains(&Permission::NetworkEgress))
            .then(|| Instance::default().lan_network(slug)),
        ports: lan.ports.clone(),
        webui: false,
    })
}

fn dry_run_capabilities(manifest: &Manifest, app_data: &Path) -> BTreeMap<String, String> {
    let provider = super::matrix_rtc::provides_matrix_rtc(manifest)
        .then(|| {
            [
                (
                    super::capabilities::MATRIX_SERVER_NAME_VAR.to_owned(),
                    "wonton.invalid".to_owned(),
                ),
                (
                    super::matrix_rtc_ca::CA_DIR_VAR.to_owned(),
                    app_data
                        .join(super::matrix_rtc_ca::CA_DIR)
                        .display()
                        .to_string(),
                ),
            ]
        })
        .into_iter()
        .flatten();
    super::dependencies::declarations(manifest)
        .flat_map(|capability| {
            [
                (
                    super::capabilities::var_url(capability),
                    format!("http://{}.wonton.internal", capability),
                ),
                (
                    super::capabilities::var_key(capability),
                    "dry-run".to_owned(),
                ),
            ]
        })
        .chain(provider)
        .collect()
}

pub fn render(
    package: &PluginPackage,
    ctx: &InstallContext,
    langs: &Languages,
) -> Result<Deployment, RenderError> {
    render_plan(package, ctx, langs).map_err(|e| explain_owner_vars(e, ctx, langs))
}

/// dry run 里声明过的共享文件夹占位：指向虚构根下的同名目录。
fn dry_run_shares(manifest: &Manifest, root: &Path) -> BTreeMap<String, PathBuf> {
    manifest
        .permissions
        .iter()
        .filter_map(|p| match p {
            Permission::StorageShare { name, .. } => {
                Some((name.clone(), root.join("shares").join(name)))
            }
            _ => None,
        })
        .collect()
}

/// dry run 里每个 Bot 的占位值：渲染和正式安装走同一条路，多 Bot 的插件每个 Bot 一套。
fn dry_run_bots(manifest: &Manifest) -> BTreeMap<String, BotVars> {
    if manifest.bots.is_none() || !manifest.permissions.contains(&Permission::MatrixBot) {
        return BTreeMap::new();
    }
    let vars = BotVars {
        model_key: manifest
            .permissions
            .contains(&Permission::Models)
            .then(|| "dry-run".into()),
        matrix: Some(MatrixVars {
            homeserver: "http://matrix.wonton.internal".into(),
            user_id: "@dry-run:wonton.invalid".into(),
            access_token: "dry-run".into(),
            allowed_users: "@owner:wonton.invalid".into(),
            home_room: "!dry-run:wonton.invalid".into(),
        }),
        mcp_url: manifest
            .permissions
            .contains(&Permission::Tools)
            .then(|| "http://mcp.wonton.internal/mcp/dry-run".into()),
    };
    BTreeMap::from([(super::bot_services::DEFAULT_BOT.to_owned(), vars)])
}

/// 一个服务的容器：Bot 服务每个 Bot 一份，全插件一份的服务只一份。
#[allow(clippy::too_many_arguments)]
fn render_service_containers(
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
        let mut mounts = Mounts {
            ctx,
            read_only_shares,
            named: package.named_volumes(),
            service: name,
            allow_docker_socket: ctx.granted.contains(&Permission::DockerSocket),
            socket_declared: manifest.permissions.contains(&Permission::DockerSocket)
                || manifest
                    .optional_permissions
                    .contains_key(&Permission::DockerSocket),
            langs,
        }
        .build(&service.volumes, binds)?;
        mounts.extend(hosted_mounts(manifest, ctx, name));
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
        };
        let mut plan = container_plan(name, service, bot, mounts, ctx, options, langs)?;
        if let Some(grant) = &ctx.lan {
            lan::publish(name, service, grant, &mut plan.body, langs)?;
        }
        if let Some(egress) = ctx.egress.as_ref()
            && matches!(&service.network_mode, None | Some(NetworkMode::Service(_)))
        {
            egress.inject(&mut plan.body, service_keys.iter().map(String::as_str));
            egress::healthcheck::apply(&mut plan.body);
        }
        plan.files = files;
        plan.store_dir = store_dir;
        plan.one_shot = one_shot;
        plan.variables = variables.clone();
        containers.push(plan);
    }
    Ok(())
}

fn render_plan(
    package: &PluginPackage,
    ctx: &InstallContext,
    langs: &Languages,
) -> Result<Deployment, RenderError> {
    let manifest = &package.manifest;
    let requirements = manifest.requirements.as_ref();
    let (services, vars, mut used) = validated_services(package, ctx, langs)?;
    let network = ctx.instance.network(&ctx.slug);
    let labels = ctx.instance.labels(&manifest.id);
    let read_only_shares = read_only_share_paths(manifest, &ctx.shares);
    let one_shot = package.one_shot_services();
    let gpu_services: &[String] = manifest.gpu.as_ref().map_or(&[], |g| &g.services);
    let mut binds = Vec::new();
    let mut containers = Vec::new();
    let service_keys: Vec<String> = services.keys().cloned().collect();
    for name in start_order(dependencies(&services), langs)? {
        let service = &services[&name];
        let mut variables = used.remove(&name).unwrap_or_default();
        check_network_mode(manifest, &name, service, langs)?;
        let gpu = ctx.gpu.as_ref().filter(|_| gpu_services.contains(&name));
        let one_shot = one_shot.contains(name.as_str());
        if one_shot && service.restart.as_deref() != Some("no") {
            return Err(service_err(
                langs,
                &name,
                crate::l10n::text(langs, "plugin-install-render-one-shot-restart", &[]),
            ));
        }
        render_service_containers(
            package,
            ctx,
            &vars,
            &service_keys,
            &name,
            service,
            one_shot,
            gpu,
            requirements,
            network.as_str(),
            &labels,
            &read_only_shares,
            &mut binds,
            &mut variables,
            &mut containers,
            langs,
        )?;
    }
    check_not_nested(&binds, langs)?;
    let mut dirs = dir_owners(&binds, &services, ctx, langs)?;
    let relays = prepare_relays(manifest, ctx, langs, &services, &mut containers, &mut dirs)?;
    for container in &mut containers {
        super::render_drift::stamp(container);
    }
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

type ServiceSet<'a> = (
    BTreeMap<String, Service>,
    ServiceVars<'a>,
    BTreeMap<String, UsedVars>,
);

/// 校验服务字段和权限（含 Bot 依赖的规则），返回服务表、变量和每个服务用到的变量。
fn validated_services<'a>(
    package: &'a PluginPackage,
    ctx: &InstallContext,
    langs: &Languages,
) -> Result<ServiceSet<'a>, RenderError> {
    let manifest = &package.manifest;
    let vars = ServiceVars::new(
        manifest,
        &ctx.app_data,
        injected_vars(manifest, ctx),
        &ctx.bots,
    );
    let mut used = BTreeMap::new();
    let services = package.services(|name| vars.get(name), &mut used, langs)?;
    if let Err(error) = super::privileges::validate_services(manifest, &services, langs) {
        return Err(service_err(langs, &error.service, error.message));
    }
    if let Some((service, bot_service)) = vars.plugin_service_on_bot(&services) {
        return Err(service_err(
            langs,
            &service,
            crate::l10n::text(
                langs,
                "plugin-install-render-bot-dependency",
                &[("service", bot_service.into())],
            ),
        ));
    }
    Ok((services, vars, used))
}

/// 中继容器和目录：宿主网络的服务各要一个。核心一侧的套接字路径放不下时在这报出来——
/// 这之后才是拉镜像、起容器，装之前就失败。
fn prepare_relays(
    manifest: &Manifest,
    ctx: &InstallContext,
    langs: &Languages,
    services: &BTreeMap<String, Service>,
    containers: &mut Vec<ContainerPlan>,
    dirs: &mut BTreeMap<PathBuf, Owner>,
) -> Result<Vec<HostRelay>, RenderError> {
    let relays = super::host_relay::needed(manifest, services);
    if let Some(problem) = super::host_relay::path_too_long(&ctx.app_data, &relays, langs) {
        return Err(RenderError::HostPath(problem));
    }
    add_relays(manifest, ctx, &relays, containers, dirs);
    Ok(relays)
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
fn check_network_mode(
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
fn read_only_share_paths(manifest: &Manifest, shares: &BTreeMap<String, PathBuf>) -> Vec<PathBuf> {
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

fn dependencies(services: &BTreeMap<String, Service>) -> BTreeMap<&str, Vec<String>> {
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

/// 按 `depends_on` 拓扑排序；同层按名字排序，保证结果确定。
pub(super) fn start_order(
    mut pending: BTreeMap<&str, Vec<String>>,
    langs: &Languages,
) -> Result<Vec<String>, RenderError> {
    for (name, deps) in &pending {
        if let Some(missing) = deps.iter().find(|d| !pending.contains_key(d.as_str())) {
            return Err(service_err(
                langs,
                name,
                crate::l10n::text(
                    langs,
                    "plugin-install-render-dependency-missing",
                    &[("missing", missing.as_str().into())],
                ),
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
            return Err(service_err(
                langs,
                &names.join(", "),
                crate::l10n::text(langs, "plugin-install-render-dependency-cycle", &[]),
            ));
        }
        for name in ready {
            pending.remove(name);
            order.push(name.to_owned());
        }
    }
    Ok(order)
}

pub(super) fn list(spec: &StringOrList) -> Vec<String> {
    match spec {
        StringOrList::One(s) => vec![s.clone()],
        StringOrList::Many(items) => items.clone(),
    }
}
