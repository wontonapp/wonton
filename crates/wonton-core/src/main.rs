//! `wonton-core`：服务端核心进程。
//!
//! - `wonton-core serve`（默认）：启动插件管理器、网关和隧道入口。
//! - `wonton-core healthcheck`：请求本机网关的健康端点，供容器健康检查使用。
//! - `wonton-core init <安装目录> <基础域名>`：正式安装时生成密钥、Tuwunel 配置和 compose 文件（`wonton_core::install`）。
//! - `wonton-core pair [--first] [IP:端口…]`：向运行中的核心要配对码，在终端里显示二维码（见 `pair_command`）。
//! - `wonton-core plugin check|pack <插件包目录>`：插件作者用的检查和打包（见 `plugin_command`）。
//! - `wonton-core image <icon.png|icon.svg|screenshot>`：核心内部用，在子进程里处理插件的图片
//!   （`wonton_core::plugin::image_worker`）。

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
use wonton_core::gateway::{self, Gateway};
use wonton_core::http::HttpClient;
use wonton_core::matrix::{Matrix, NoticeRoom};
use wonton_core::mcp;
use wonton_core::oidc::Hydra;
use wonton_core::plugin::PluginManager;
use wonton_core::plugin::catalog::Catalog;
use wonton_core::plugin::docker::Engine;
use wonton_core::plugin::image_worker::{self, ImageWorker};
use wonton_core::plugin::lifecycle::LifecycleConfig;
use wonton_core::plugin::store::Store;
use wonton_core::push::Push;
use wonton_core::secret;
use wonton_core::tunnel::{self, Tunnel, TunnelEntry};

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
    if args.get(1).map(String::as_str) == Some(image_worker::COMMAND) {
        return image_worker::child(&args[2..]);
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

async fn serve() -> anyhow::Result<()> {
    tracing::info!(commit = %BUILD_COMMIT, "核心启动");
    let cfg = Config::from_env()?;
    let engine = Engine::connect()?;
    let docker = engine.check_server().await.context("连接 Docker")?;
    tracing::info!(version = %docker.version, arch = %docker.arch, "Docker 已连接");

    let token_path = cfg.core_state_dir().join("owner-token");
    let owner_token = secret::load_or_create(&token_path)
        .with_context(|| format!("读取主人令牌 {}", token_path.display()))?;
    tracing::info!(path = %token_path.display(), "主人令牌已就绪");

    let http = HttpClient::new();
    let hydra = connect_hydra(&cfg, &http).await?;
    let matrix = connect_matrix(&cfg, &http).await?;

    let store = Store::open(cfg.core_state_dir().join("plugins"))?;
    let lifecycle = LifecycleConfig {
        public: cfg.public.clone(),
        apps_dir: cfg.apps_dir(),
        shares_dir: cfg.shares_dir(),
        core_container: cfg.core_container.clone(),
        tz: cfg.tz.clone(),
        puid: cfg.puid,
        pgid: cfg.pgid,
        install_timeout: cfg.install_timeout,
        owner: cfg.owner.username.clone(),
        gateway_port: cfg.listen.port(),
        instance: cfg.instance.clone(),
    };
    let images =
        ImageWorker::current().context("找不到核心自己的可执行文件（处理插件图片要用）")?;
    let manager = Arc::new(
        PluginManager::new(
            engine,
            store,
            http.clone(),
            hydra.as_ref().map(|(h, _)| h.clone()),
            lifecycle,
            images,
        )
        .with_matrix(matrix.clone()),
    );
    manager.restore().await.context("恢复已安装的插件")?;
    preinstall(&cfg, manager.clone())?;

    let bus = mcp_bus(&cfg, manager.clone(), matrix.clone())?;
    let tunnel = bind_tunnel(&cfg).await?;
    let push_path = cfg.core_state_dir().join("push.json");
    let push = Push::open(&push_path, tunnel.devices.clone())
        .with_context(|| format!("打开推送登记表 {}", push_path.display()))?;
    let gateway = Arc::new(
        Gateway::new(
            cfg.public.clone(),
            cfg.owner.clone(),
            owner_token,
            manager,
            hydra,
            http,
            tunnel.clone(),
        )
        .with_matrix(matrix)
        .with_push(Some(Arc::new(push)))
        .with_mcp(Some(bus.endpoint())),
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

/// 在后台装上还没装的系统插件（要拉镜像、等健康，不挡着网关启动），再为模型网关的使用方对一次账：凭据的值在
/// 核心，网关的数据丢了也能补回来。系统插件目录配了却不是目录时启动失败。
fn preinstall(cfg: &Config, manager: Arc<PluginManager>) -> anyhow::Result<()> {
    let dir = cfg.system_plugins.clone();
    if let Some(dir) = &dir
        && !dir.is_dir()
    {
        bail!("WONTON_SYSTEM_PLUGINS 指向的 {} 不是目录", dir.display());
    }
    tokio::spawn(async move {
        if let Some(dir) = dir
            && let Err(e) = manager.ensure_system(&dir).await
        {
            tracing::error!(error = %e, "预装系统插件失败");
        }
        for (plugin, error) in manager.reconcile_models().await {
            tracing::error!(%plugin, %error, "模型网关凭据对账失败");
        }
    });
    Ok(())
}

/// MCP 总线（ADR-0016）：确认卡片发进和主人的通知房间，审计写在核心的状态目录里。插件目录配了却不是目录时启动
/// 失败；没接 Matrix 时敏感的调用如实报“没法请主人确认”。
fn mcp_bus(
    cfg: &Config,
    manager: Arc<PluginManager>,
    matrix: Option<Arc<Matrix>>,
) -> anyhow::Result<Arc<mcp::Bus>> {
    if let Some(dir) = &cfg.catalog
        && !dir.is_dir()
    {
        bail!("WONTON_CATALOG 指向的 {} 不是目录", dir.display());
    }
    let notifier: Arc<dyn mcp::Notifier> = match matrix {
        Some(matrix) => {
            let saved = cfg.core_state_dir().join("matrix-notice-room");
            Arc::new(NoticeRoom::new(matrix, &cfg.owner.username, saved))
        }
        None => Arc::new(mcp::NoMatrix),
    };
    let confirmations = Arc::new(mcp::Confirmations::new(notifier, cfg.confirmation_ttl));
    let catalog = cfg.catalog.clone().map(Catalog::new);
    let audit = cfg.core_state_dir().join("audit.jsonl");
    Ok(mcp::Bus::new(manager, catalog, confirmations, audit))
}

/// 读取已配对设备和隧道私钥，绑定隧道 endpoint。绑不上就启动失败：没有隧道，App 连不回家。
async fn bind_tunnel(cfg: &Config) -> anyhow::Result<Tunnel> {
    let devices = Devices::open(cfg.core_state_dir().join("devices.json"))?;
    let key = tunnel::load_or_create_key(&cfg.core_state_dir().join("tunnel-key"))?;
    let endpoint = tunnel::bind(key, cfg.tunnel_port).await?;
    tracing::info!(
        endpoint_id = %endpoint.id(),
        sockets = ?endpoint.bound_sockets(),
        devices = devices.list().len(),
        "隧道入口已启动"
    );
    Ok(Tunnel::new(Arc::new(devices), endpoint, cfg.tunnel_port))
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

fn init(args: &[String]) -> anyhow::Result<()> {
    let [dir, server_name] = args else {
        bail!("用法：wonton-core init <安装目录> <基础域名>");
    };
    let report = wonton_core::install::init(std::path::Path::new(dir), server_name)?;
    for path in &report.created {
        println!("已生成 {}", path.display());
    }
    for path in &report.kept {
        println!("沿用 {}", path.display());
    }
    Ok(())
}
