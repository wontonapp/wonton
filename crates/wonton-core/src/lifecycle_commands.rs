//! 一次性生命周期命令：`healthcheck`、`init`、`updater`、`manual-rollback` 的入口。

use anyhow::{Context, bail};
use hyper::Method;
use std::time::Duration;
use wonton_core::config::Config;
use wonton_core::http::HttpClient;
use wonton_core::upgrade::docker_host::DockerHost;
use wonton_core::upgrade::state::Layout;
use wonton_core::upgrade::{manual, updater};

use super::Engine;

pub(super) async fn healthcheck() -> anyhow::Result<()> {
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
pub(super) fn check_owner_before_install() -> anyhow::Result<()> {
    let get = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    wonton_core::config::check_owner_settings(&get)?;
    if let Some(pool) = get(wonton_core::plugin::network_pool::VAR) {
        pool.parse::<wonton_core::plugin::network_pool::NetworkPool>()
            .map_err(anyhow::Error::msg)
            .context("环境变量 WONTON_PLUGIN_NETWORK_POOL 不合法")?;
    }
    if let Some(data_dir) = get("WONTON_DATA_DIR") {
        let state = std::path::Path::new(&data_dir).join("core");
        let configured = get("WONTON_OWNER_NAME");
        wonton_core::owner::check_before_install(&state, configured.as_deref())?;
    }
    Ok(())
}

pub(super) async fn run_updater(args: &[String]) -> anyhow::Result<()> {
    let [install_dir, data_dir] = args else {
        bail!("用法：wonton-core updater <安装目录> <数据目录>");
    };
    let host = DockerHost::new(
        Engine::connect()?,
        Box::new({
            let data_dir: std::path::PathBuf = data_dir.into();
            move || wonton_core::plugin::store::read_owner_languages(&data_dir)
        }),
    );
    let layout = Layout {
        install_dir: install_dir.into(),
        data_dir: data_dir.into(),
    };
    // 升级器写进状态文件的文字（App 详情里显示）按主人的语言（ADR-0043）。
    let langs = wonton_core::plugin::store::read_owner_languages(&layout.data_dir);
    updater::run(&host, layout, langs, updater::observe_period()).await
}

pub(super) async fn manual_rollback(args: &[String]) -> anyhow::Result<()> {
    let rollback = manual::parse(args)?;
    let host = DockerHost::new(
        Engine::connect()?,
        Box::new({
            let data_dir = rollback.layout.data_dir.clone();
            move || wonton_core::plugin::store::read_owner_languages(&data_dir)
        }),
    );
    rollback.run(&host).await
}

/// 第二个参数是基础域名：旧版本的升级器照这个格式调新镜像的 `init`，所以留着，`init` 不再用它。Matrix 服务器名只在
/// 第一次安装时由 `WONTON_MATRIX_SERVER_NAME` 指定。
pub(super) fn init(args: &[String]) -> anyhow::Result<()> {
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
