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
use super::hostdir::Owner;
use super::interpolate::Vars;
use super::manifest::{Access, Manifest, Permission, SecretSpec, Sso};
use super::mounts::{Mounts, check_not_nested, dir_owners, mount_targets};
use super::package::{PackageError, PluginPackage};
use super::secrets::secret_var;

/// Docker 资源上的标签：标明归属哪个插件，卸载时按标签清理。
pub const LABEL_PLUGIN: &str = "wonton.plugin";
pub const LABEL_SERVICE: &str = "wonton.service";

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error(transparent)]
    Package(#[from] PackageError),
    #[error("服务 `{service}`：{message}")]
    Service { service: String, message: String },
    /// 插件声明了核心还兑现不了的权限，又引用了这个权限的变量。
    #[error("{0}")]
    Unsupported(String),
}

pub(super) fn service_err(service: &str, message: impl Into<String>) -> RenderError {
    RenderError::Service {
        service: service.to_owned(),
        message: message.into(),
    }
}

/// 注入给 OIDC 客户端的配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcVars {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub userinfo_url: String,
}

/// 安装环境：由核心配置和本次安装分配的资源决定。
#[derive(Debug, Clone)]
pub struct InstallContext {
    pub slug: String,
    /// 插件私有数据目录（宿主路径）。
    pub app_data: PathBuf,
    /// 已授权的共享文件夹：名称 → 宿主路径。
    pub shares: BTreeMap<String, PathBuf>,
    /// 插件网页的公开地址（不带末尾斜杠）。
    pub app_url: Option<String>,
    pub oidc: Option<OidcVars>,
    /// 核心生成的插件密钥（`secrets` 里的 `generate` 项）：名字 → 值。
    pub secrets: BTreeMap<String, String>,
    pub tz: String,
    pub puid: u32,
    pub pgid: u32,
    /// 额外写进每个容器 `/etc/hosts` 的条目（核心不在容器里运行时，用来让插件访问网关）。
    pub extra_hosts: Vec<String>,
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
}

pub fn share_var(name: &str) -> String {
    format!(
        "WONTON_SHARE_{}",
        name.to_ascii_uppercase().replace('-', "_")
    )
}

/// 核心为插件提供的插值变量。
pub fn injected_vars(manifest: &Manifest, ctx: &InstallContext) -> Vars {
    let mut vars = Vars::new();
    let mut set = |k: &str, v: String| {
        vars.insert(k.to_owned(), v);
    };
    set("WONTON_APP_ID", manifest.id.to_string());
    set("WONTON_APP_DATA", ctx.app_data.display().to_string());
    set("TZ", ctx.tz.clone());
    set("PUID", ctx.puid.to_string());
    set("PGID", ctx.pgid.to_string());
    for (name, path) in &ctx.shares {
        set(&share_var(name), path.display().to_string());
    }
    if let Some(url) = &ctx.app_url {
        set("WONTON_APP_URL", url.clone());
    }
    for (name, value) in &ctx.secrets {
        set(&secret_var(name), value.clone());
    }
    if let Some(oidc) = &ctx.oidc {
        set("WONTON_OIDC_ISSUER", oidc.issuer.clone());
        set("WONTON_OIDC_CLIENT_ID", oidc.client_id.clone());
        set("WONTON_OIDC_CLIENT_SECRET", oidc.client_secret.clone());
        set("WONTON_OIDC_AUTH_URL", oidc.auth_url.clone());
        set("WONTON_OIDC_TOKEN_URL", oidc.token_url.clone());
        set("WONTON_OIDC_USERINFO_URL", oidc.userinfo_url.clone());
    }
    vars
}

/// 用占位环境渲染一遍，在不接触 Docker 的情况下检查插件包能否安装。
pub fn dry_run(package: &PluginPackage) -> Result<Deployment, RenderError> {
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
    let ctx = InstallContext {
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
        secrets: manifest
            .secrets
            .iter()
            .filter(|(_, spec)| matches!(spec, SecretSpec::Generate(_)))
            .map(|(name, _)| (name.clone(), "dry-run".to_owned()))
            .collect(),
        slug,
        tz: "UTC".into(),
        puid: 1000,
        pgid: 1000,
        extra_hosts: Vec::new(),
    };
    render(package, &ctx)
}

pub fn render(package: &PluginPackage, ctx: &InstallContext) -> Result<Deployment, RenderError> {
    let manifest = &package.manifest;
    let vars = ServiceVars::new(manifest, &ctx.app_data, injected_vars(manifest, ctx));
    let services = package
        .services(|name| vars.get(name))
        .map_err(|e| explain(manifest, e))?;
    if let Some((service, bot_service)) = vars.plugin_service_on_bot(&services) {
        return Err(service_err(
            &service,
            format!("全插件一份的服务不能依赖 Bot 服务 `{bot_service}`：Bot 服务每个 Bot 一份"),
        ));
    }
    let network = format!("wonton-{}", ctx.slug);
    let labels = HashMap::from([(LABEL_PLUGIN.to_owned(), manifest.id.to_string())]);
    let read_only_shares: Vec<PathBuf> = ctx
        .shares
        .iter()
        .filter(|(name, _)| !share_writable(manifest, name))
        .map(|(_, path)| path.clone())
        .collect();
    let one_shot = package.one_shot_services();
    let mut binds = Vec::new();
    let mut containers = Vec::new();
    for name in start_order(&services)? {
        let service = &services[&name];
        check_network_mode(manifest, &name, service)?;
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
        }
        .build(&service.volumes, &mut binds)?;
        let targets = mount_targets(service, &mounts);
        let files =
            configs::resolve(package, service, vars.get(&name), &targets).map_err(|e| match e {
                ConfigError::Package(e) => explain(manifest, e),
                ConfigError::Rule(m) => service_err(&name, m),
            })?;
        let endpoint = (network.as_str(), vars.aliases(&name));
        let mut plan = container_plan(&name, service, mounts, ctx, endpoint, &labels)?;
        plan.files = files;
        plan.one_shot = one_shot;
        containers.push(plan);
    }
    check_not_nested(&binds)?;
    let dirs = dir_owners(&binds, &services, ctx)?;
    let volumes = package
        .named_volumes()
        .iter()
        .map(|v| volume_name(&ctx.slug, v))
        .collect();
    Ok(Deployment {
        network,
        labels,
        volumes,
        dirs,
        containers,
    })
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

/// 引用了没有定义的变量，而变量属于插件声明了、核心还兑现不了的权限（模型网关、Matrix Bot 接入之前）：
/// 如实报“尚未支持”，而不是让人以为插件忘了声明权限。
fn explain(manifest: &Manifest, error: PackageError) -> RenderError {
    let pending = error.missing_var().and_then(|var| {
        let permission = if var.starts_with("WONTON_MODEL_") {
            Permission::Models
        } else if var.starts_with("WONTON_MATRIX_") {
            Permission::MatrixBot
        } else {
            return None;
        };
        manifest
            .permissions
            .contains(&permission)
            .then(|| format!("权限 {permission} 尚未支持：{var} 要等核心接入后才有值"))
    });
    match pending {
        Some(message) => RenderError::Unsupported(message),
        None => error.into(),
    }
}

fn share_writable(manifest: &Manifest, share: &str) -> bool {
    manifest.permissions.iter().any(
        |p| matches!(p, Permission::StorageShare { name, access: Access::Write } if name == share),
    )
}

/// 容器名 `wonton-<slug>--<服务名>`。slug 由单个 `-` 连接的小写字母数字段组成，不含 `--`，也不以 `-`
/// 结尾，所以第一个 `--` 就是分隔符，不同插件的容器不会重名。只用一个 `-` 时会重名：`app.foo` 的服务
/// `bar-web` 和 `app.foo.bar` 的服务 `web` 都是 `wonton-app-foo-bar-web`。
pub fn container_name(slug: &str, service: &str) -> String {
    format!("wonton-{slug}--{service}")
}

/// 命名卷 `wonton-<slug>--<卷名>`，理由同容器名。
pub fn volume_name(slug: &str, volume: &str) -> String {
    format!("wonton-{slug}--{volume}")
}

/// 按 `depends_on` 拓扑排序；同层按名字排序，保证结果确定。
fn start_order(services: &BTreeMap<String, Service>) -> Result<Vec<String>, RenderError> {
    let mut pending: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (name, service) in services {
        let deps: Vec<String> = service
            .depends_on
            .as_ref()
            .map(|d| d.entries().into_iter().map(|(n, _)| n).collect())
            .unwrap_or_default();
        if let Some(missing) = deps.iter().find(|d| !services.contains_key(*d)) {
            return Err(service_err(
                name,
                format!("depends_on 引用了不存在的服务 `{missing}`"),
            ));
        }
        pending.insert(name, deps);
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

/// `endpoint` 是插件网络和服务在其中的别名。
fn container_plan(
    name: &str,
    service: &Service,
    mounts: Vec<Mount>,
    ctx: &InstallContext,
    (network, aliases): (&str, Vec<String>),
    labels: &HashMap<String, String>,
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
    all_labels.extend(labels.clone());
    all_labels.insert(LABEL_SERVICE.to_owned(), name.to_owned());

    let depends_on = service
        .depends_on
        .as_ref()
        .map(|d| d.entries())
        .unwrap_or_default()
        .into_iter()
        .map(|(dep, cond)| (container_name(&ctx.slug, &dep), cond))
        .collect();
    // 没有网络的服务不接插件网络。
    let networking_config =
        (service.network_mode != Some(NetworkMode::None)).then(|| NetworkingConfig {
            endpoints_config: Some(HashMap::from([(
                network.to_owned(),
                EndpointSettings {
                    aliases: Some(aliases),
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
        host_config: Some(host_config(name, service, mounts, ctx, network)?),
        networking_config,
        ..Default::default()
    };
    Ok(ContainerPlan {
        service: name.to_owned(),
        name: container_name(&ctx.slug, name),
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
    network: &str,
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
    let isolated = service.network_mode == Some(NetworkMode::None);
    Ok(HostConfig {
        network_mode: Some(if isolated { "none" } else { network }.to_owned()),
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
        extra_hosts: (!isolated && !ctx.extra_hosts.is_empty()).then(|| ctx.extra_hosts.clone()),
        ..Default::default()
    })
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
