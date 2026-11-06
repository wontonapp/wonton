//! `wonton-core`：服务端核心进程。
//!
//! - `wonton-core serve`（默认）：启动插件管理器、网关和隧道入口。
//! - `wonton-core healthcheck`：请求本机网关的健康端点，供容器健康检查使用。
//! - `wonton-core init <安装目录> <基础域名>`：正式安装时生成密钥、Matrix 服务器名、Tuwunel 配置和 compose 文件
//!   （`wonton_core::install`）。
//! - `wonton-core pair [--first] [IP:端口…]`：向运行中的核心要配对码，在终端里显示二维码（见 `pair_command`）。
//! - `wonton-core plugin check|pack <插件包目录>`：插件作者用的检查和打包（见 `plugin_command`）。
//! - `wonton-core image <icon.png|icon.svg|screenshot>`：核心内部用，在子进程里处理插件的图片
//!   （`wonton_core::plugin::image_worker`）。
//! - `wonton-core lan-probe`：核心内部用，在宿主网络里的一次性容器里读宿主的路由和网卡
//!   （`wonton_core::plugin::lan::probe`）。
//! - `wonton-core egress-proxy-check`：查代理连不连得上，安装脚本和核心都在出口网络上的一次性容器里跑它
//!   （`wonton_core::plugin::egress::check`）。
//! - `wonton-core updater <安装目录> <数据目录>`：核心内部用，升级器容器（`wonton_core::upgrade::updater`）。
//! - `wonton-core compose-images`：升级器在新镜像里执行，列出这个版本要用的其他镜像。
//! - `wonton-core manual-rollback …`：`install.sh` 手动退回时在目标镜像里执行（`wonton_core::upgrade::manual`）。

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use hyper::Method;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;
use wonton_core::config::Config;
use wonton_core::device::Devices;
use wonton_core::files::{CoreHost, Files, FilesConfig};
use wonton_core::gateway::{self, Gateway};
use wonton_core::http::HttpClient;
use wonton_core::matrix::{Matrix, NoticeRoom};
use wonton_core::mcp;
use wonton_core::mcp::audit::Audit;
use wonton_core::oidc::Hydra;
use wonton_core::owner::OwnerStore;
use wonton_core::plugin::PluginManager;
use wonton_core::plugin::docker::Engine;
use wonton_core::plugin::egress::check;
use wonton_core::plugin::egress::settings::Egress;
use wonton_core::plugin::gpu::HostPaths;
use wonton_core::plugin::hostdir::Owner;
use wonton_core::plugin::image_worker::{self, ImageWorker};
use wonton_core::plugin::lan::{self, Lan};
use wonton_core::plugin::lifecycle::LifecycleConfig;
use wonton_core::plugin::names::Instance;
use wonton_core::plugin::preinstall::Backoff;
use wonton_core::plugin::store::Store;
use wonton_core::push::Push;
use wonton_core::secret;
use wonton_core::tunnel::{self, Tunnel, TunnelEntry};
use wonton_core::upgrade::docker_host::DockerHost;
use wonton_core::upgrade::service::{UpgradeConfig, Upgrades};
use wonton_core::upgrade::state::Layout;
use wonton_core::upgrade::{compose_images, manual, updater};

mod pair_command;
mod plugin_command;

/// 构建它的提交号：开发镜像构建时由 `deploy/dev/core/dev.sh` 传入，直接 `cargo build` 时没有。
const BUILD_COMMIT: &str = match option_env!("WONTON_BUILD_COMMIT") {
    Some(commit) => commit,
    None => "unknown",
};

fn main() -> ExitCode {
    // 处理图片的子进程不起 tokio 运行时，也不初始化日志（日志写在标准输出上，会混进结果）。
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some(image_worker::COMMAND) => return image_worker::child(&args[2..]),
        // 检测容器的结果打印在标准输出上，同样不初始化日志。
        Some(lan::probe::COMMAND) => return lan::probe::command(),
        Some(check::COMMAND) => return check::command(),
        Some(compose_images::COMMAND) => return compose_images::command(),
        _ => {}
    }
    run()
}

#[tokio::main]
async fn run() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let command = std::env::args().nth(1);
    if command.as_deref() == Some("plugin") {
        return plugin_command::run(&std::env::args().skip(2).collect::<Vec<_>>()).await;
    }
    let result = match command.as_deref() {
        None | Some("serve") => serve().await,
        Some("healthcheck") => healthcheck().await,
        Some("pair") => pair_command::run(std::env::args().skip(2).collect()).await,
        Some("init") => init(&std::env::args().skip(2).collect::<Vec<_>>()),
        Some("updater") => run_updater(&std::env::args().skip(2).collect::<Vec<_>>()).await,
        Some("manual-rollback") => {
            manual_rollback(&std::env::args().skip(2).collect::<Vec<_>>()).await
        }
        Some("--version") => {
            println!("wonton-core {} ({BUILD_COMMIT})", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => Err(anyhow::anyhow!(
            "未知命令 {other}；用法：wonton-core [serve|healthcheck|init|pair|plugin]"
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

fn check_host_dev(cfg: &Config) -> anyhow::Result<()> {
    if let Some(dev) = &cfg.host_dev
        && !dev.is_dir()
    {
        bail!(
            "WONTON_HOST_DEV 指向的 {} 不是目录：核心在容器里时要把宿主的 /dev 只读挂载到这里",
            dev.display()
        );
    }
    Ok(())
}

async fn serve() -> anyhow::Result<()> {
    tracing::info!(commit = %BUILD_COMMIT, "核心启动");
    let cfg = Config::from_env()?.lock_owner()?;
    tracing::info!(owner = %cfg.username, "主人的用户名已核对");
    let owner = OwnerStore::open(&cfg.core_state_dir())?;
    check_host_dev(&cfg)?;
    let engine = Engine::connect()?;
    let docker = engine.check_server().await.context("连接 Docker")?;
    tracing::info!(version = %docker.version, arch = %docker.arch, "Docker 已连接");

    let token_path = cfg.core_state_dir().join("owner-token");
    let owner_token = secret::load_or_create(&token_path)
        .with_context(|| format!("读取主人令牌 {}", token_path.display()))?;
    tracing::info!(path = %token_path.display(), "主人令牌已就绪");

    let upgrades = Arc::new(Upgrades::new(
        upgrade_config(&cfg),
        Arc::new(DockerHost::new(engine.clone())),
    ));
    let http = HttpClient::new();
    let system_http =
        HttpClient::for_system_services(&engine, cfg.core_container.as_deref()).await?;
    let hydra = connect_hydra(&cfg, &system_http).await?;
    let matrix = connect_matrix(&cfg, &system_http).await?;

    let store = Store::open(cfg.core_state_dir().join("plugins"))?;
    let images =
        Arc::new(ImageWorker::current().context("找不到核心自己的可执行文件（处理图片要用）")?);
    let manager = Arc::new(
        PluginManager::new(
            engine,
            store,
            http.clone(),
            hydra.as_ref().map(|(h, _)| h.clone()),
            lifecycle_config(&cfg, owner.clone()),
            images.clone(),
        )
        .with_matrix(matrix.clone())
        .with_host(HostPaths::new(cfg.host_dev.clone()))
        .with_lan(lan_state(&cfg))
        .with_egress(Egress::new(
            cfg.egress_proxy.clone(),
            cfg.egress_proxy_detected.clone(),
        ))
        .with_store(cfg.store.clone())
        .with_upgrade_freeze(upgrades.frozen()),
    );
    manager.spawn_registry_mirror_probe();
    manager.restore().await.context("恢复已安装的插件")?;
    spawn_plugin_monitors(&manager);
    upgrades.spawn();
    preinstall(&cfg, manager.clone(), upgrades.clone())?;

    let audit = Arc::new(Audit::new(cfg.core_state_dir().join("audit.jsonl")));
    let notices = matrix.clone().map(|matrix| {
        let saved = cfg.core_state_dir().join("matrix-notice-room");
        Arc::new(NoticeRoom::new(matrix, &cfg.username, saved))
    });
    let bus = mcp_bus(&cfg, manager.clone(), notices.clone(), audit.clone());
    bus.attach_help(help_docs(&cfg, manager.clone(), upgrades.clone()));
    let host = Arc::new(CoreHost::new(manager.clone(), bus.upstreams(), notices));
    let files = open_files(&cfg, host, http.clone(), audit, images)?;
    bus.attach_files(files.clone());
    files.spawn_sweeper();
    let tunnel = bind_tunnel(&cfg, &manager).await?;
    let push = open_push(&cfg, &tunnel, &manager)?;
    let gateway = Arc::new(
        Gateway::new(
            cfg.public.clone(),
            owner,
            owner_token,
            manager,
            hydra,
            http,
            tunnel.clone(),
        )
        .with_system_http(system_http)
        .with_matrix(matrix)
        .with_push(Some(Arc::new(push)))
        .with_mcp(Some(bus.endpoint()))
        .with_files(Some(files))
        .with_tusd(cfg.tusd.clone())
        .with_upgrades(Some(upgrades)),
    );
    let app = gateway::router(gateway);
    let listener = TcpListener::bind(cfg.listen)
        .await
        .with_context(|| format!("监听 {}", cfg.listen))?;
    let entry = TunnelEntry::spawn(&tunnel, app.clone());
    tracing::info!(
        listen = %cfg.listen,
        url = %cfg.public.origin(cfg.public.core_host()),
        "网关已启动"
    );
    let served = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await
    .context("网关异常退出");
    entry.shutdown().await;
    served
}

fn spawn_plugin_monitors(manager: &Arc<PluginManager>) {
    manager.spawn_lan_monitor();
    manager.spawn_egress_monitor();
    manager.spawn_upgrade_checks();
    manager.spawn_oom_watch();
    manager.spawn_container_watch();
}

fn lifecycle_config(cfg: &Config, owner: Arc<OwnerStore>) -> LifecycleConfig {
    LifecycleConfig {
        public: cfg.public.clone(),
        data_dir: cfg.data_dir.clone(),
        apps_dir: cfg.apps_dir(),
        shares_dir: cfg.shares_dir(),
        core_container: cfg.core_container.clone(),
        tz: cfg.tz.clone(),
        puid: cfg.puid,
        pgid: cfg.pgid,
        install_timeout: cfg.install_timeout,
        owner: cfg.username.clone(),
        profile: owner,
        gateway_port: cfg.listen.port(),
        plugin_slice: cfg.plugin_slice.clone(),
        instance: cfg.instance.clone(),
    }
}

fn upgrade_config(cfg: &Config) -> UpgradeConfig {
    UpgradeConfig {
        source: cfg.core_source.clone(),
        install_dir: cfg.install_dir.clone(),
        data_dir: cfg.data_dir.clone(),
        core_container: cfg.core_container.clone(),
        has_instance: cfg.instance != Instance::default(),
    }
}

fn lan_state(cfg: &Config) -> Lan {
    let detector = Lan::for_core(cfg.core_container.clone());
    match &cfg.lan_addresses {
        Some(addresses) => {
            tracing::info!(
                ?addresses,
                "局域网端口只开在 WONTON_LAN_ADDRESSES 指定的地址上"
            );
            detector.with_fixed(addresses.clone())
        }
        None => detector,
    }
}

async fn connect_hydra(cfg: &Config, http: &HttpClient) -> anyhow::Result<Option<(Hydra, String)>> {
    let Some(oidc) = &cfg.oidc else {
        tracing::warn!("没有配置身份提供方，声明 sso: oidc 的插件将无法安装");
        return Ok(None);
    };
    let hydra = Hydra::new(http.clone(), oidc);
    hydra
        .wait_ready(Duration::from_secs(90))
        .await
        .context("等待身份提供方就绪")?;
    tracing::info!(admin = %oidc.admin_url, "身份提供方已就绪");
    Ok(Some((hydra, oidc.public_url.clone())))
}

/// 等 Tuwunel 就绪、准备好核心的管理账号。连不上就启动失败：配置了 Matrix 却用不了，不降级运行。
async fn connect_matrix(cfg: &Config, http: &HttpClient) -> anyhow::Result<Option<Arc<Matrix>>> {
    let Some(config) = &cfg.matrix else {
        tracing::warn!("没有配置 Matrix（WONTON_MATRIX_URL），App 登录 Matrix 和 Bot 插件都不可用");
        return Ok(None);
    };
    let matrix = Matrix::connect(config, &cfg.core_state_dir(), http.clone())
        .await
        .context("连接 Tuwunel")?;
    tracing::info!(
        url = %config.url,
        server_name = %matrix.server_name(),
        admin = %matrix.admin_user_id(),
        "Tuwunel 已就绪"
    );
    Ok(Some(Arc::new(matrix)))
}

/// 在后台预装（要拉镜像、等健康，不挡着网关启动）：先装上还没装的系统插件，再为模型网关的使用方对一次账（凭据的
/// 值在核心，网关的数据丢了也能补回来），最后装默认插件：默认 Agent 要用模型网关。目录配了却不是目录时启动失败。
/// 核心升级没结束时都先等着（ADR-0025）：回滚后插件的记录和容器要对得上。
fn preinstall(
    cfg: &Config,
    manager: Arc<PluginManager>,
    upgrades: Arc<Upgrades>,
) -> anyhow::Result<()> {
    let (system, defaults) = (cfg.system_plugins.clone(), cfg.default_plugins.clone());
    for (name, dir) in [
        ("WONTON_SYSTEM_PLUGINS", &system),
        ("WONTON_DEFAULT_PLUGINS", &defaults),
    ] {
        if let Some(dir) = dir
            && !dir.is_dir()
        {
            bail!("{name} 指向的 {} 不是目录", dir.display());
        }
    }
    let backoff = Backoff::default();
    tokio::spawn(async move {
        upgrades.wait_released().await;
        if let Some(dir) = system
            && let Err(e) = manager.ensure_system(&dir, backoff).await
        {
            tracing::error!(error = %e, "预装系统插件失败");
        }
        for (plugin, error) in manager.reconcile_models().await {
            tracing::error!(%plugin, %error, "模型网关凭据对账失败");
        }
        if let Some(dir) = defaults
            && let Err(e) = manager.ensure_defaults(&dir, backoff).await
        {
            tracing::error!(error = %e, "预装默认插件失败");
        }
    });
    Ok(())
}

/// MCP 总线（ADR-0016）：确认卡片发进和主人的通知房间。没接 Matrix 时敏感的调用如实报“没法请主人确认”。
fn mcp_bus(
    cfg: &Config,
    manager: Arc<PluginManager>,
    notices: Option<Arc<NoticeRoom>>,
    audit: Arc<Audit>,
) -> Arc<mcp::Bus> {
    let notifier: Arc<dyn mcp::Notifier> = match notices {
        Some(notices) => notices,
        None => Arc::new(mcp::NoMatrix),
    };
    let confirmations = Arc::new(
        mcp::Confirmations::new(notifier, cfg.confirmation_ttl).with_windows(
            cfg.confirmation_window,
            cfg.core_state_dir().join("confirmation-windows.json"),
        ),
    );
    mcp::Bus::new(manager, confirmations, audit)
}

fn open_push(cfg: &Config, tunnel: &Tunnel, manager: &PluginManager) -> anyhow::Result<Push> {
    let path = cfg.core_state_dir().join("push.json");
    Push::open(&path, tunnel.devices.clone())
        .and_then(|push| push.with_proxy(manager.core_proxy()))
        .with_context(|| format!("打开推送登记表 {}", path.display()))
}

/// 帮助文档（ADR-0034）。版本号和升级接口报的是同一个。
fn help_docs(
    cfg: &Config,
    manager: Arc<PluginManager>,
    upgrades: Arc<Upgrades>,
) -> Arc<wonton_core::help::Help> {
    let version: wonton_core::help::CoreVersion = Arc::new(move || {
        let upgrades = upgrades.clone();
        Box::pin(async move { upgrades.running_version().await })
    });
    wonton_core::help::Help::new(cfg.docs.clone(), manager, version)
}

/// 文件服务（ADR-0020）。读不出编号记录、建不了 `transfers` 就启动失败。
fn open_files(
    cfg: &Config,
    host: Arc<CoreHost>,
    http: HttpClient,
    audit: Arc<Audit>,
    images: Arc<ImageWorker>,
) -> anyhow::Result<Arc<Files>> {
    let config = FilesConfig {
        dir: cfg.core_state_dir().join("files"),
        shares_dir: cfg.shares_dir(),
        owner: Owner {
            uid: cfg.puid,
            gid: cfg.pgid,
        },
    };
    let files = Files::open(&config, host, http, audit, images).context("打开文件服务")?;
    Ok(Arc::new(files))
}

/// 读取已配对设备和隧道私钥，绑定隧道 endpoint。绑不上就启动失败：没有隧道，App 连不回家。
async fn bind_tunnel(cfg: &Config, manager: &PluginManager) -> anyhow::Result<Tunnel> {
    let devices = Devices::open(cfg.core_state_dir().join("devices.json"))?;
    let key = tunnel::load_or_create_key(&cfg.core_state_dir().join("tunnel-key"))?;
    let endpoint = tunnel::bind(key, cfg.tunnel_port).await?;
    tracing::info!(
        endpoint_id = %endpoint.id(),
        sockets = ?endpoint.bound_sockets(),
        devices = devices.list().len(),
        "隧道入口已启动"
    );
    let addresses = manager.lan_addresses();
    let tunnel = Tunnel::new(
        Arc::new(devices),
        endpoint,
        cfg.tunnel_port,
        addresses.clone(),
    );
    tunnel::spawn_announcing(tunnel.endpoint.clone(), tunnel.port, addresses);
    Ok(tunnel)
}

async fn shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("无法注册 SIGTERM 处理");
    tokio::select! {
        _ = ctrl_c => {}
        _ = term.recv() => {}
    }
    tracing::info!("收到退出信号");
}

async fn healthcheck() -> anyhow::Result<()> {
    let cfg = Config::from_env()?;
    let url = format!("http://127.0.0.1:{}/_wonton/health", cfg.listen.port());
    let request = hyper::Request::builder()
        .method(Method::GET)
        .uri(&url)
        .header(hyper::header::HOST, cfg.public.core_host())
        .body(axum::body::Body::empty())?;
    let response = tokio::time::timeout(Duration::from_secs(5), HttpClient::new().send(request))
        .await
        .context("健康检查超时")??;
    if !response.status().is_success() {
        bail!("健康检查失败：HTTP {}", response.status());
    }
    Ok(())
}

/// 安装脚本把要写进 `.env` 的主人设置传给 `init`：不合规则或者和记录对不上时当场报错，不用等容器反复重启。`init` 也
/// 由一键升级执行，那时什么都不传，这里什么都不查。
fn check_owner_before_install() -> anyhow::Result<()> {
    let get = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    wonton_core::config::check_owner_settings(&get)?;
    if let Some(data_dir) = get("WONTON_DATA_DIR") {
        let state = std::path::Path::new(&data_dir).join("core");
        let configured = get("WONTON_OWNER_NAME");
        wonton_core::owner::check_before_install(&state, configured.as_deref())?;
    }
    Ok(())
}

async fn run_updater(args: &[String]) -> anyhow::Result<()> {
    let [install_dir, data_dir] = args else {
        bail!("用法：wonton-core updater <安装目录> <数据目录>");
    };
    let host = DockerHost::new(Engine::connect()?);
    let layout = Layout {
        install_dir: install_dir.into(),
        data_dir: data_dir.into(),
    };
    updater::run(&host, layout, updater::observe_period()).await
}

async fn manual_rollback(args: &[String]) -> anyhow::Result<()> {
    let rollback = manual::parse(args)?;
    rollback.run(&DockerHost::new(Engine::connect()?)).await
}

/// 第二个参数是基础域名：旧版本的升级器照这个格式调新镜像的 `init`，所以留着，`init` 不再用它。Matrix 服务器名只在
/// 第一次安装时由 `WONTON_MATRIX_SERVER_NAME` 指定。
fn init(args: &[String]) -> anyhow::Result<()> {
    let [dir, _base_domain] = args else {
        bail!("用法：wonton-core init <安装目录> <基础域名>");
    };
    check_owner_before_install()?;
    let requested = std::env::var("WONTON_MATRIX_SERVER_NAME")
        .ok()
        .filter(|v| !v.is_empty());
    let report = wonton_core::install::init(std::path::Path::new(dir), requested.as_deref())?;
    for path in &report.created {
        println!("已生成 {}", path.display());
    }
    for path in &report.kept {
        println!("沿用 {}", path.display());
    }
    Ok(())
}
