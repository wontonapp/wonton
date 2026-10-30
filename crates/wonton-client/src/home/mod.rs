//! 和家里服务器的连接：本机设备身份、iroh 隧道客户端，以及 WebView 和 HTTP 客户端用的本地代理
//! （ADR-0004）。
//!
//! 状态放在 watch 通道里，壳经订阅拿到 [`HomeViewModel`]。网络操作都在核心自己的 tokio 运行时里执行，
//! `dispatch` 不阻塞。

mod storage;
mod ticket;

use std::net::Ipv4Addr;
use std::sync::{Arc, Weak};
use std::time::Instant;

use iroh::endpoint::presets;
use iroh::{Endpoint, SecretKey};
use tokio::runtime::Handle;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use url::Url;
use wonton_protocol::{CoreError, HomeFailure, HomePath, HomeStatus, HomeViewModel, ProxySettings};
use wonton_tunnel::{
    LocalProxy, PathInfo, PathKind, ProxyAccess, ProxyCredential, TunnelClient, TunnelError,
    TunnelStatus,
};

pub(crate) use storage::{Storage, write_private};
use ticket::Ticket;

/// iroh 的网络配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    /// 用 n0 的公共中继和地址发现（开发和内测期，见 ADR-0004 的调整）。
    Internet,
    /// 只按连接信息里的直连地址连，不连中继、不做地址发现：本机和局域网测试用。
    DirectOnly,
}

pub(crate) struct HomeLink {
    runtime: Handle,
    storage: Storage,
    device_key: SecretKey,
    network: Network,
    view: watch::Sender<HomeViewModel>,
    /// 当前用的连接信息（原文）。
    ticket: std::sync::Mutex<Option<String>>,
    /// 当前连接信息里核心自己的地址，聊天从这里取 Matrix 登录令牌。
    core_url: std::sync::Mutex<Option<Url>>,
    /// 当前的连接。换服务器时整个替换；锁保证同一时刻只有一次建连或检查。
    link: Mutex<Option<Link>>,
}

/// 到一台服务器的连接：endpoint、隧道客户端和本地代理同生共死。
struct Link {
    endpoint: Endpoint,
    client: TunnelClient,
    proxy: LocalProxy,
    watcher: JoinHandle<()>,
}

impl HomeLink {
    /// 读取设备密钥；上次连过服务器时自动重连。
    pub(crate) fn new(
        runtime: Handle,
        storage: Storage,
        network: Network,
    ) -> Result<Arc<Self>, CoreError> {
        let device_key = storage.device_key()?;
        tracing::info!(device_id = %device_key.public(), "本机设备身份");
        let saved = storage.ticket()?;
        let view = watch::Sender::new(HomeViewModel {
            device_id: device_key.public().to_string(),
            server_id: None,
            status: HomeStatus::NotConfigured,
            path: None,
            proxy: None,
            web_url: None,
            pairing: None,
        });
        let home = Arc::new(Self {
            runtime,
            storage,
            device_key,
            network,
            view,
            ticket: std::sync::Mutex::new(None),
            core_url: std::sync::Mutex::new(None),
            link: Mutex::new(None),
        });
        if let Some(text) = saved {
            // 只有校验过的连接信息才会被保存，读出来不合法说明存储坏了。
            let ticket = Ticket::parse(&text).map_err(|reason| CoreError::Storage {
                reason: format!("保存的连接信息已损坏：{reason}"),
            })?;
            home.start(text, ticket);
        }
        Ok(home)
    }

    pub(crate) fn view(&self) -> HomeViewModel {
        self.view.borrow().clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<HomeViewModel> {
        self.view.subscribe()
    }

    /// `ConnectHome`：校验并保存连接信息，然后在后台建连。和当前用的连接信息相同时什么都不做：
    /// 连接断了会在下一次使用时自动重连，重建反而会让正在用本地代理的 WebView 断开。
    pub(crate) fn connect(self: &Arc<Self>, text: &str) -> Result<(), CoreError> {
        let ticket = Ticket::parse(text).map_err(|reason| CoreError::InvalidInput { reason })?;
        if self.lock_ticket().as_deref() == Some(text) {
            return Ok(());
        }
        self.storage.save_ticket(text)?;
        self.start(text.to_owned(), ticket);
        Ok(())
    }

    /// `AppForegrounded`：在后台检查连接，必要时重连。
    pub(crate) fn foregrounded(self: &Arc<Self>) {
        self.runtime.spawn(self.clone().resume());
    }

    fn lock_ticket(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.ticket
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 核心自己的地址；还没有连接信息时为空。
    pub(crate) fn core_url(&self) -> Option<Url> {
        self.core_url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn start(self: &Arc<Self>, text: String, ticket: Ticket) {
        *self.lock_ticket() = Some(text);
        *self
            .core_url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(ticket.core_url());
        self.view.send_modify(|view| {
            view.server_id = Some(ticket.server_id().to_string());
            view.web_url = ticket.web_url().map(str::to_owned);
            view.status = HomeStatus::Connecting;
            view.path = None;
            view.proxy = None;
        });
        self.runtime.spawn(self.clone().establish(ticket));
    }

    async fn establish(self: Arc<Self>, ticket: Ticket) {
        let mut current = self.link.lock().await;
        if let Some(old) = current.take() {
            old.close().await;
        }
        match self.open(&ticket).await {
            Ok(link) => {
                let client = link.client.clone();
                *current = Some(link);
                drop(current);
                // 主动建连：壳打开网页之前，状态就已经是准的。
                self.check(&client).await;
            }
            Err(message) => self.view.send_modify(|view| {
                view.status = HomeStatus::Disconnected {
                    reason: HomeFailure::TransportUnavailable,
                    message,
                };
            }),
        }
    }

    async fn open(self: &Arc<Self>, ticket: &Ticket) -> Result<Link, String> {
        let builder = match self.network {
            Network::Internet => Endpoint::builder(presets::N0),
            Network::DirectOnly => Endpoint::builder(presets::Minimal),
        };
        let endpoint = builder
            .secret_key(self.device_key.clone())
            .bind()
            .await
            .map_err(|e| format!("绑定网络失败：{e}"))?;
        let client = TunnelClient::new(endpoint.clone(), ticket.server_addr());
        let credential = ProxyCredential::generate();
        let domain = ticket.base_domain().to_owned();
        let access = ProxyAccess::Credential {
            credential: credential.clone(),
            domain: domain.clone(),
        };
        let proxy = LocalProxy::bind(client.clone(), (Ipv4Addr::LOCALHOST, 0).into(), access)
            .await
            .map_err(|e| format!("本地代理启动失败：{e}"))?;
        let settings = ProxySettings {
            host: Ipv4Addr::LOCALHOST.to_string(),
            port: proxy.local_addr().port(),
            username: credential.username().to_owned(),
            password: credential.password().to_owned(),
            domain,
        };
        self.view.send_modify(|view| view.proxy = Some(settings));
        let watcher = self
            .runtime
            .spawn(watch_status(Arc::downgrade(self), client.clone()));
        Ok(Link {
            endpoint,
            client,
            proxy,
            watcher,
        })
    }

    async fn resume(self: Arc<Self>) {
        let started = Instant::now();
        let client = {
            let current = self.link.lock().await;
            let Some(link) = current.as_ref() else {
                return;
            };
            // 挂起期间网络可能变了（换 Wi-Fi、切蜂窝），让 iroh 重新检查网卡和路径。
            link.endpoint.network_change().await;
            link.client.clone()
        };
        // 挂起久了，服务端已经放弃这条连接，手机却收不到关闭帧：限时确认，没有回应就立刻换新连接，
        // 不等 30 秒的空闲超时（见 ADR-0004 的真机实测）。
        if let Err(err) = client.revalidate().await {
            tracing::info!("隧道检查失败：{err}");
        }
        self.update_path(&client);
        tracing::info!(
            elapsed = ?started.elapsed(),
            status = %client.status(),
            "回到前台，隧道检查完成"
        );
    }

    /// 和服务端做一次往返（必要时重连），更新路径。状态由 `watch_status` 跟着隧道客户端更新。
    async fn check(&self, client: &TunnelClient) {
        if let Err(err) = client.ping().await {
            tracing::info!("隧道检查失败：{err}");
        }
        self.update_path(client);
    }

    fn update_path(&self, client: &TunnelClient) {
        let path = client.path().map(path_view);
        self.view.send_modify(|view| view.path = path);
    }
}

impl Link {
    async fn close(self) {
        self.watcher.abort();
        self.proxy.shutdown().await;
        self.client.close();
        self.endpoint.close().await;
    }
}

/// 把隧道客户端的状态变化写进 ViewModel。只持有弱引用，核心释放后自动结束。
async fn watch_status(home: Weak<HomeLink>, client: TunnelClient) {
    let mut status = client.watch_status();
    loop {
        let current = status.borrow_and_update().clone();
        let Some(home) = home.upgrade() else {
            return;
        };
        let path = client.path().map(path_view);
        home.view.send_modify(|view| {
            view.status = status_view(&current);
            view.path = path;
        });
        drop(home);
        if status.changed().await.is_err() {
            return;
        }
    }
}

fn status_view(status: &TunnelStatus) -> HomeStatus {
    match status {
        TunnelStatus::Idle | TunnelStatus::Connecting => HomeStatus::Connecting,
        TunnelStatus::Connected => HomeStatus::Connected,
        TunnelStatus::Disconnected(err) => HomeStatus::Disconnected {
            reason: failure(err),
            message: err.to_string(),
        },
        TunnelStatus::Closed => HomeStatus::Disconnected {
            reason: HomeFailure::TransportUnavailable,
            message: "隧道客户端已关闭".to_owned(),
        },
    }
}

fn failure(err: &TunnelError) -> HomeFailure {
    match err {
        TunnelError::Unauthorized => HomeFailure::Unauthorized,
        TunnelError::UpstreamUnavailable => HomeFailure::UpstreamUnavailable,
        TunnelError::Protocol { .. } => HomeFailure::Protocol,
        _ => HomeFailure::TransportUnavailable,
    }
}

fn path_view(path: PathInfo) -> HomePath {
    HomePath {
        relay: path.kind == PathKind::Relay,
        rtt_ms: u32::try_from(path.rtt.as_millis()).unwrap_or(u32::MAX),
    }
}
