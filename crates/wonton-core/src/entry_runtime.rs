//! 隔离入口监听、续期和家名状态通知。失败不替换旧证书，也不改成明文入口。

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, ensure};
use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::{self, Next};
use axum::response::Response;
use tokio::net::{TcpListener, TcpStream};

use crate::entry_settings::EntrySettings;
use crate::gateway::Gateway;
use crate::matrix::NoticeRoom;
use crate::plugin::docker::Engine;

pub const FRPC_IMAGE: &str =
    "fatedier/frpc:v0.71.1@sha256:7a0a18bc9c83dc106a7e5213a890f521affdf90d595829c56b0ed8481377d50e";

/// 主网关不接受入口网络的连接；同样的规则不依赖 Cookie 或请求头。
pub struct MainListener {
    inner: TcpListener,
    excluded: Vec<IpAddr>,
}

impl MainListener {
    pub async fn bind(listen: SocketAddr) -> anyhow::Result<Self> {
        let excluded = match std::env::var("WONTON_EXCLUDE_LISTEN") {
            Ok(host) if !host.is_empty() => tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .map(|addr| addr.ip())
                .collect(),
            _ => Vec::new(),
        };
        Ok(Self {
            inner: TcpListener::bind(listen).await?,
            excluded,
        })
    }
}

impl axum::serve::Listener for MainListener {
    type Io = TcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, addr) = axum::serve::Listener::accept(&mut self.inner).await;
            if stream
                .local_addr()
                .is_ok_and(|addr| !self.excluded.contains(&addr.ip()))
            {
                return (stream, addr);
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

pub async fn bind() -> anyhow::Result<Option<TcpListener>> {
    let Some(listen) = std::env::var("WONTON_ENTRY_LISTEN")
        .ok()
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    let address = tokio::net::lookup_host(&listen)
        .await?
        .find(SocketAddr::is_ipv4)
        .context("入口监听没有 IPv4 地址")?;
    Ok(Some(TcpListener::bind(address).await?))
}

pub fn router(gateway: Arc<Gateway>) -> axum::Router {
    crate::gateway::entry_router(gateway).layer(middleware::from_fn_with_state(
        "entry.wonton.internal:8449".to_owned(),
        trusted_entry,
    ))
}

async fn trusted_entry(State(authority): State<String>, req: Request, next: Next) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|addr| addr.0.ip());
    let trusted = match tokio::net::lookup_host(&authority).await {
        Ok(mut addresses) => peer.is_some_and(|peer| addresses.any(|addr| addr.ip() == peer)),
        Err(error) => {
            tracing::error!(%error, "入口地址解析失败");
            false
        }
    };
    if !trusted {
        return axum::response::IntoResponse::into_response((
            hyper::StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({"errcode":"M_FORBIDDEN","error":"untrusted entry"})),
        ));
    }
    next.run(req).await
}

pub fn spawn(
    install_dir: &Path,
    data_dir: &Path,
    engine: Engine,
    core_container: String,
    notices: Option<Arc<NoticeRoom>>,
) -> anyhow::Result<()> {
    let Some(settings) = EntrySettings::load(install_dir)? else {
        return Ok(());
    };
    let install_dir = install_dir.to_owned();
    let data_dir = data_dir.to_owned();
    tokio::spawn(async move {
        let mut hourly = tokio::time::interval(Duration::from_secs(3600));
        let mut previous = None;
        let mut hours = 0;
        loop {
            hourly.tick().await;
            check_home(&settings, &data_dir, &notices, &mut previous).await;
            if hours % 24 == 0 && !settings.external_certificate {
                match settings
                    .certificate(&install_dir, &data_dir, engine.clone())
                    .await
                {
                    Ok(()) => {
                        let loaded = async {
                            crate::entry_compose::prepare_permissions(&install_dir)?;
                            reload_entry(&engine, &core_container).await
                        }
                        .await;
                        if let Err(error) = loaded {
                            tracing::error!(%error, "入口证书热加载失败");
                            notify(
                                &notices,
                                "entry-certificate-reload-failed",
                                &[("error", error.to_string().as_str())],
                            )
                            .await;
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "入口证书续期失败，保留原证书");
                        notify(
                            &notices,
                            "entry-certificate-renew-failed",
                            &[("error", error.to_string().as_str())],
                        )
                        .await;
                    }
                }
            }
            hours += 1;
        }
    });
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NameState {
    Active,
    Suspended,
    Unknown,
}

async fn check_home(
    settings: &EntrySettings,
    data_dir: &Path,
    notices: &Option<Arc<NoticeRoom>>,
    previous: &mut Option<NameState>,
) {
    let Some(home) = &settings.registration else {
        return;
    };
    let result = async {
        settings
            .service(data_dir)?
            .context("家名服务配置缺失")?
            .status(&home.home)
            .await
    }
    .await;
    let (state, id, args) = match &result {
        Ok(status) if status.suspended => (
            NameState::Suspended,
            "entry-name-suspended",
            vec![("domain", status.domain.as_str())],
        ),
        Ok(status) => (
            NameState::Active,
            "entry-name-active",
            vec![("domain", status.domain.as_str())],
        ),
        Err(error) => {
            tracing::error!(%error, "家名状态查询失败，状态未知");
            (
                NameState::Unknown,
                "entry-name-unknown",
                vec![("domain", settings.domain.as_str())],
            )
        }
    };
    if *previous != Some(state) && (state != NameState::Active || previous.is_some()) {
        notify(notices, id, &args).await;
    }
    *previous = Some(state);
}

async fn reload_entry(engine: &Engine, core: &str) -> anyhow::Result<()> {
    let info = engine
        .inspect_container(core)
        .await?
        .context("核心容器不存在")?;
    let labels = info
        .config
        .and_then(|c| c.labels)
        .context("核心缺少 Compose 标签")?;
    let project = labels
        .get("com.docker.compose.project")
        .context("核心缺少 Compose 项目")?;
    let resources = engine
        .labeled("com.docker.compose.project", project, |labels| {
            labels
                .get("com.docker.compose.service")
                .is_some_and(|s| s == "entry")
        })
        .await?;
    ensure!(resources.containers.len() == 1, "入口容器数量不对");
    engine
        .kill_container(&resources.containers[0], "SIGUSR2")
        .await?;
    Ok(())
}

async fn notify(notices: &Option<Arc<NoticeRoom>>, id: &str, args: &[(&str, &str)]) {
    let Some(notices) = notices else {
        return;
    };
    let text = crate::l10n::text(
        &(notices.languages)(),
        id,
        &args
            .iter()
            .map(|&(key, value)| (key, value.into()))
            .collect::<Vec<_>>(),
    );
    if let Err(error) = notices
        .send(&serde_json::json!({"msgtype":"m.text","body":text}))
        .await
    {
        tracing::error!(%error, "入口状态通知失败");
    }
}
