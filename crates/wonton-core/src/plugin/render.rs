//! 渲染：把插件包和安装环境变成要交给 Docker 的部署计划。
//!
//! 这里是纯函数，不访问 Docker。注入的变量见 `docs/plugin-spec.md` 的“安装时注入的变量”。

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};

use bollard::models::{
    ContainerCreateBody, EndpointSettings, HealthConfig, HostConfig, Mount, MountType,
    NetworkingConfig, RestartPolicy, RestartPolicyNameEnum,
};

use super::compose::{
    DependCondition, Healthcheck, LongVolume, NetworkMode, PullPolicy, Service, StringOrList,
    VolumeEntry, VolumeType, parse_bytes, parse_duration,
};
use super::configs::{self, ConfigFile};
use super::interpolate::Vars;
use super::manifest::{Access, Manifest, Permission, Sso};
use super::package::{PackageError, PluginPackage};

/// Docker 资源上的标签：标明归属哪个插件，卸载时按标签清理。
pub const LABEL_PLUGIN: &str = "wonton.plugin";
pub const LABEL_SERVICE: &str = "wonton.service";

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error(transparent)]
    Package(#[from] PackageError),
    #[error("服务 `{service}`：{message}")]
    Service { service: String, message: String },
}

fn service_err(service: &str, message: impl Into<String>) -> RenderError {
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
}

#[derive(Debug, Clone)]
pub struct Deployment {
    pub network: String,
    pub labels: HashMap<String, String>,
    /// 要创建的 Docker 命名卷。
    pub volumes: Vec<String>,
    /// 要预先创建并交给 PUID:PGID 的宿主目录（应用数据目录下的绑定挂载源）。
    pub dirs: Vec<PathBuf>,
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
    let vars = injected_vars(manifest, ctx);
    let services = package.services(&vars)?;
    let network = format!("wonton-{}", ctx.slug);
    let labels = HashMap::from([(LABEL_PLUGIN.to_owned(), manifest.id.to_string())]);
    let read_only_shares: Vec<PathBuf> = ctx
        .shares
        .iter()
        .filter(|(name, _)| !share_writable(manifest, name))
        .map(|(_, path)| path.clone())
        .collect();
    let mut binds = Vec::new();
    let mut containers = Vec::new();
    for name in start_order(&services)? {
        let service = &services[&name];
        check_network_mode(manifest, &name, service)?;
        let mounts = Mounts {
            ctx,
            read_only_shares: &read_only_shares,
            named: package.named_volumes(),
            service: &name,
        }
        .build(&service.volumes, &mut binds)?;
        let files = configs::resolve(package, service, &vars, &mount_targets(service, &mounts))
            .map_err(|m| service_err(&name, m))?;
        let mut plan = container_plan(&name, service, mounts, ctx, &network, &labels)?;
        plan.files = files;
        containers.push(plan);
    }
    check_not_nested(&binds)?;
    let mut dirs: Vec<PathBuf> = binds
        .into_iter()
        .map(|(_, path)| path)
        .filter(|path| path.starts_with(&ctx.app_data))
        .collect();
    dirs.sort();
    dirs.dedup();
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

/// 服务所有挂载的目标（卷、绑定挂载、`tmpfs`）。
fn mount_targets(service: &Service, mounts: &[Mount]) -> Vec<PathBuf> {
    let tmpfs = service.tmpfs.as_ref().map(list).unwrap_or_default();
    mounts
        .iter()
        .filter_map(|m| m.target.clone())
        .chain(
            tmpfs
                .into_iter()
                .map(|entry| entry.split(':').next().unwrap_or_default().to_owned()),
        )
        .map(|target| Path::new(&target).components().collect())
        .collect()
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

/// 同一插件的绑定挂载源不能嵌套：能写外层目录的容器可以把内层的源换成软链接，Docker 下次启动容器时
/// 跟随它，就把宿主上的任意目录挂了进来。同一个目录挂进多个服务没有问题。
fn check_not_nested(binds: &[(String, PathBuf)]) -> Result<(), RenderError> {
    for (service, inner) in binds {
        let outer = binds
            .iter()
            .find(|(_, outer)| outer != inner && inner.starts_with(outer));
        if let Some((outer_service, outer)) = outer {
            return Err(service_err(
                service,
                format!(
                    "宿主路径 `{}` 在服务 `{outer_service}` 挂载的 `{}` 里面；同一插件的绑定挂载不能嵌套\
                     （容器能把内层目录换成软链接，逃出应用数据目录），请改用并列的子目录",
                    inner.display(),
                    outer.display()
                ),
            ));
        }
    }
    Ok(())
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

fn container_plan(
    name: &str,
    service: &Service,
    mounts: Vec<Mount>,
    ctx: &InstallContext,
    network: &str,
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
                    aliases: Some(vec![name.to_owned()]),
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

fn list(spec: &StringOrList) -> Vec<String> {
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

/// 把 compose 的 `volumes` 变成 Docker 挂载，并检查宿主路径是否在允许范围内。
struct Mounts<'a> {
    ctx: &'a InstallContext,
    read_only_shares: &'a [PathBuf],
    named: &'a std::collections::BTreeSet<String>,
    service: &'a str,
}

impl Mounts<'_> {
    /// `binds` 收集每个绑定挂载的（服务名，宿主路径），供跨服务的嵌套检查。
    fn build(
        &self,
        entries: &[VolumeEntry],
        binds: &mut Vec<(String, PathBuf)>,
    ) -> Result<Vec<Mount>, RenderError> {
        entries
            .iter()
            .map(|entry| {
                let long = match entry {
                    VolumeEntry::Long(long) => long.clone(),
                    VolumeEntry::Short(text) => self.parse_short(text)?,
                };
                self.mount(long, binds)
            })
            .collect()
    }

    fn parse_short(&self, text: &str) -> Result<LongVolume, RenderError> {
        let parts: Vec<&str> = text.split(':').collect();
        let (source, target, mode) = match parts.as_slice() {
            [target] => (None, *target, ""),
            [source, target] => (Some(*source), *target, ""),
            [source, target, mode] => (Some(*source), *target, *mode),
            _ => return Err(self.err(format!("卷 `{text}` 格式不合法"))),
        };
        let modes: Vec<&str> = mode.split(',').filter(|m| !m.is_empty()).collect();
        if let Some(bad) = modes
            .iter()
            .find(|m| !matches!(**m, "ro" | "rw" | "z" | "Z"))
        {
            return Err(self.err(format!("卷选项 `{bad}` 不受支持")));
        }
        let kind = match source {
            None => VolumeType::Volume,
            Some(s) if s.starts_with('/') => VolumeType::Bind,
            Some(s) if s.starts_with('.') || s.starts_with('~') => {
                return Err(self.err(format!(
                    "不支持相对路径 `{s}`，请使用 ${{WONTON_APP_DATA}} 下的路径"
                )));
            }
            Some(_) => VolumeType::Volume,
        };
        Ok(LongVolume {
            kind,
            source: source.map(str::to_owned),
            target: target.to_owned(),
            read_only: modes.contains(&"ro"),
        })
    }

    fn mount(
        &self,
        volume: LongVolume,
        binds: &mut Vec<(String, PathBuf)>,
    ) -> Result<Mount, RenderError> {
        if !volume.target.starts_with('/') {
            return Err(self.err(format!("挂载目标 `{}` 必须是绝对路径", volume.target)));
        }
        let source = match (volume.kind, volume.source) {
            (VolumeType::Tmpfs, _) => None,
            (VolumeType::Volume, None) => None,
            (VolumeType::Volume, Some(name)) => {
                if !self.named.contains(&name) {
                    return Err(self.err(format!("命名卷 `{name}` 没有在顶层 volumes 中声明")));
                }
                Some(volume_name(&self.ctx.slug, &name))
            }
            (VolumeType::Bind, None) => return Err(self.err("绑定挂载缺少 source")),
            (VolumeType::Bind, Some(path)) => {
                let path = self.check_bind(&path, volume.read_only)?;
                binds.push((self.service.to_owned(), path.clone()));
                Some(path.display().to_string())
            }
        };
        let typ = match volume.kind {
            VolumeType::Bind => MountType::BIND,
            VolumeType::Volume => MountType::VOLUME,
            VolumeType::Tmpfs => MountType::TMPFS,
        };
        Ok(Mount {
            target: Some(volume.target),
            source,
            typ: Some(typ),
            read_only: Some(volume.read_only),
            ..Default::default()
        })
    }

    /// 宿主路径只能是应用数据目录（或其子目录）或已授权共享文件夹的根目录；只读共享必须以只读挂载。
    /// 返回去掉 `.`、重复的 `/` 和末尾 `/` 之后的路径。
    fn check_bind(&self, raw: &str, read_only: bool) -> Result<PathBuf, RenderError> {
        let raw = Path::new(raw);
        if raw.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(self.err(format!("宿主路径 `{}` 不能包含 ..", raw.display())));
        }
        let path: PathBuf = raw.components().collect();
        if path.starts_with(&self.ctx.app_data) {
            return Ok(path);
        }
        let Some(root) = self.ctx.shares.values().find(|root| path.starts_with(root)) else {
            return Err(self.err(format!(
                "宿主路径 `{}` 不在应用数据目录或已授权的共享文件夹里",
                path.display()
            )));
        };
        // 共享文件夹由多个插件共用，有写权限的插件能把其中的子目录换成软链接。
        if path != *root {
            return Err(self.err(format!(
                "共享文件夹只能整个挂载，不能只挂其中的 `{}`",
                path.display()
            )));
        }
        if self.read_only_shares.contains(root) && !read_only {
            return Err(self.err(format!(
                "共享文件夹 `{}` 只授权了读，必须以 :ro 挂载",
                root.display()
            )));
        }
        Ok(path)
    }

    fn err(&self, message: impl Into<String>) -> RenderError {
        service_err(self.service, message)
    }
}
