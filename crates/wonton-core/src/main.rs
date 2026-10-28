//! `wonton-core`：服务端核心进程。
//!
//! - `wonton-core serve`（默认）：启动插件管理器和网关。
//! - `wonton-core healthcheck`：请求本机网关的健康端点，供容器健康检查使用。

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use hyper::Method;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;
use wonton_core::config::Config;
use wonton_core::gateway::{self, Gateway};
use wonton_core::http::HttpClient;
use wonton_core::oidc::Hydra;
use wonton_core::plugin::PluginManager;
use wonton_core::plugin::docker::Engine;
use wonton_core::plugin::lifecycle::LifecycleConfig;
use wonton_core::plugin::store::Store;
use wonton_core::secret;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let command = std::env::args().nth(1);
    let result = match command.as_deref() {
        None | Some("serve") => serve().await,
        Some("healthcheck") => healthcheck().await,
        Some("--version") => {
            println!("wonton-core {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => Err(anyhow::anyhow!(
            "未知命令 {other}；用法：wonton-core [serve|healthcheck]"
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
    let cfg = Config::from_env()?;
    let engine = Engine::connect()?;
    let arch = engine.host_arch().await.context("连接 Docker")?;
    tracing::info!(%arch, "Docker 已连接");

    let token_path = cfg.core_state_dir().join("owner-token");
    let owner_token = secret::load_or_create(&token_path)
        .with_context(|| format!("读取主人令牌 {}", token_path.display()))?;
    tracing::info!(path = %token_path.display(), "主人令牌已就绪");

    let http = HttpClient::new();
    let hydra = match &cfg.oidc {
        Some(oidc) => {
            let hydra = Hydra::new(http.clone(), oidc);
            hydra
                .wait_ready(Duration::from_secs(90))
                .await
                .context("等待身份提供方就绪")?;
            tracing::info!(admin = %oidc.admin_url, "身份提供方已就绪");
            Some((hydra, oidc.public_url.clone()))
        }
        None => {
            tracing::warn!("没有配置身份提供方，声明 sso: oidc 的插件将无法安装");
            None
        }
    };

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
    };
    let manager = Arc::new(PluginManager::new(
        engine,
        store,
        http.clone(),
        hydra.as_ref().map(|(h, _)| h.clone()),
        lifecycle,
    ));
    manager.restore().await.context("恢复已安装的插件")?;

    let gateway = Arc::new(Gateway::new(
        cfg.public.clone(),
        cfg.owner.clone(),
        owner_token,
        manager,
        hydra,
        http,
    ));
    let listener = TcpListener::bind(cfg.listen)
        .await
        .with_context(|| format!("监听 {}", cfg.listen))?;
    tracing::info!(
        listen = %cfg.listen,
        url = %cfg.public.origin(cfg.public.core_host()),
        "网关已启动"
    );
    axum::serve(
        listener,
        gateway::router(gateway).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await
    .context("网关异常退出")
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
