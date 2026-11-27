//! 渲染：把插件包和安装环境变成要交给 Docker 的部署计划。
//!
//! 这里是纯函数，不访问 Docker。注入的变量见 `docs/plugin-spec.md` 的“安装时注入的变量”。

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Duration;

use bollard::models::ContainerCreateBody;

pub(super) use super::bot_services::{self, BotVars, ServiceVars};
pub(super) use super::capabilities;
use super::compose::{DependCondition, PullPolicy, Service, StringOrList};
use super::configs::ConfigFile;
pub(super) use super::dependencies;
use super::device::DeviceGrant;
use super::egress::EgressVars;
use super::gpu::GpuGrant;
pub(super) use super::host_relay::{self, HostRelay};
use super::hostdir::Owner;
use super::interpolate::UsedVars;
use super::lan::LanGrant;
use super::languages::Languages;
use super::manifest::Permission;
pub(super) use super::matrix_rtc;
pub(super) use super::matrix_rtc_ca;
pub(super) use super::models;
use super::mounts::{check_not_nested, dir_owners};
use super::names::Instance;
use super::package::{PackageError, PluginPackage};
pub(super) use super::private_homes;
pub(super) use super::requirements;
pub(super) use super::settings;
use super::vars::injected_vars;
pub use super::vars::{MatrixVars, ModelVars, OidcVars, OwnerVars};
use services::{
    check_network_mode, dependencies, prepare_relays, read_only_share_paths,
    render_service_containers,
};

pub use dry_run::dry_run;
pub(super) use dry_run::dry_run_context;
use dry_run::explain_owner_vars;

/// 容器是插件的哪个服务。
pub const LABEL_SERVICE: &str = "wonton.service";
/// 容器是哪个 Bot 的：只有 Bot 服务（`bots.services`）的容器有。
pub const LABEL_BOT: &str = "wonton.bot";

mod dry_run;
mod services;
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
    /// 宿主网络中继的套接字目录。
    pub relay_dir: PathBuf,
    /// 核心管理的配置文件目录（宿主路径），`read_only` 服务的 `configs` 从这里挂进容器。
    pub config_store: PathBuf,
    /// 已授权的共享文件夹：名称 → 宿主路径。
    pub shares: BTreeMap<String, PathBuf>,
    /// 插件网页的公开地址（不带末尾斜杠）。
    pub app_url: Option<String>,
    /// 注入的规范对外地址（`WONTON_EXTERNAL_URL`，ADR-0089 第 5 节）；只有开得出入口的插件有。
    pub external_url: Option<String>,
    pub oidc: Option<OidcVars>,
    /// 声明了 `webui.sso`（`oidc` 或 `proxy`）时：主人的用户名和邮箱。这些插件本来就能从登录里得到它们，其余的
    /// 插件（包括未审核的）不多拿一份主人的个人信息。
    pub owner: Option<OwnerVars>,
    /// 声明了 `matrix.bot` 时：核心代 Bot 登录后签发的身份。只有一个 Bot（没声明 `bots`）的插件在
    /// 插件级变量里注入它；声明了 `bots` 的按 Bot 注入，见 `bots`。
    pub matrix: Option<MatrixVars>,
    /// 声明了 `bots` 时每个 Bot 的 Matrix 身份、模型凭据和总线地址（`bots.services` 里的服务用）。
    pub bots: BTreeMap<String, BotVars>,
    pub(crate) homes: BTreeMap<String, super::private_homes::HomeMount>,
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
    /// 主人选的宿主设备（ADR-0036）；没申请设备类权限时为空。
    pub devices: Option<DeviceGrant>,
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
            .field("external_url", &self.external_url)
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
            .field("devices", &self.devices)
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

pub fn render(
    package: &PluginPackage,
    ctx: &InstallContext,
    langs: &Languages,
) -> Result<Deployment, RenderError> {
    render_plan(package, ctx, langs).map_err(|e| explain_owner_vars(e, ctx, langs))
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
    let dirs = dir_owners(&binds, &services, ctx, langs)?;
    let relays = prepare_relays(manifest, ctx, langs, &services, &mut containers)?;
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
