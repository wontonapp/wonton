//! 和家里服务器的连接：本机设备身份、iroh 隧道客户端，以及 WebView 和 HTTP 客户端用的本地代理
//! （ADR-0004）。
//!
//! 状态放在 watch 通道里，壳经订阅拿到 [`HomeViewModel`]。网络操作都在核心自己的 tokio 运行时里执行，
//! `dispatch` 不阻塞。

mod background;
mod bind;
mod check;
mod connection;
mod invite;
mod link;
mod pairing;
mod quality;
mod rebuild;
mod refresh;
mod storage;
mod ticket;

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use iroh::{Endpoint, SecretKey, Watcher};
use tokio::runtime::Handle;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{
    ConnectionCheck, ConnectionDetails, ConnectionNotice, CoreError, DeviceNetwork, HomeFailure,
    HomePath, HomeStatus, HomeViewModel, ProxySettings, PushMethod,
};
use wonton_tunnel::relays::{FALLBACK_AFTER, RelayTable, spawn_tier_fallback};
use wonton_tunnel::{
    LocalProxy, PathInfo, PathKind, ProxyAccess, ProxyCredential, TunnelClient, TunnelError,
    TunnelStatus,
};

use crate::l10n::{CATALOG, language};
use crate::scope::reuse;
use check::keep_checking;
use connection::{LONG_OUTAGE, Tracker};
use link::Link;
use quality::Quality;
pub(crate) use storage::{Storage, write_private};
use ticket::Ticket;

/// iroh 的网络配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    /// 中继表从连接信息来（ADR-0048），地址发现照常；连接信息里一个中继都没有时用内置 n0。
    Internet,
    /// 只按连接信息里的直连地址连，不连中继、不做地址发现：本机和局域网测试用。
    DirectOnly,
    /// 只给本机行为测试：按 ticket 连自签测试中继，不做外网地址发现。
    #[cfg(feature = "test-hooks")]
    TestRelay,
}

/// 网络变了之后，现有连接限时确认：给 iroh 留出迁移路径的时间，不一上来就断流（ADR-0032）。
const NETWORK_CHANGE_DEADLINE: Duration = Duration::from_secs(4);
/// 系统的网络回调一次变化会连着来好几个，等这么久合成一次。
const NETWORK_CHANGE_SETTLE: Duration = Duration::from_millis(300);
/// 网页媒体停下后继续保活这么久，盖住换曲的一瞬和暂停后马上回来。
const MEDIA_RELEASE_GRACE: Duration = Duration::from_secs(60);
/// 网络变化后，隔这么久还没连上就换一次握手。初始包同时发给所有候选地址，服务端按先到的那份认路径；那个地址要是有去
/// 无回，之后的重传都白费，只有新握手才可能换路径（Android 模拟器上实测：`10.0.2.2` 的回包源地址是 127.0.0.1，到不了
/// iroh，这次握手挂满 12 秒，下一次 9 毫秒连上）。只换这两次，之后留给最后一次走完建连时限：中继冷启动的握手本来就慢。
const NETWORK_CHANGE_REDIALS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(3)];
/// 发包数、丢包数、上次确认连上的时间和路径延迟每个包都在变：这些最多每秒进一次主 ViewModel，
/// 状态、路径类型和提示照常立刻推。没有这个间隔，隧道上每走一个包，主 ViewModel 都会整份重推
/// 一遍（iOS 模拟器实测约每秒 300 次）。
const STATS_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) struct HomeLink {
    runtime: Handle,
    hub: Arc<wonton_tunnel::sharing::ProxyHub>,
    storage: Storage,
    device_key: SecretKey,
    network: Network,
    view: watch::Sender<HomeViewModel>,
    ticket: std::sync::Mutex<Option<String>>,
    core_url: std::sync::Mutex<Option<Url>>,
    link: Mutex<Option<Link>>,
    lifecycle_client: std::sync::Mutex<Option<TunnelClient>>,
    tracker: std::sync::Mutex<Tracker>,
    quality: std::sync::Mutex<Quality>,
    recount: watch::Sender<()>,
    /// 上次把每包变化的计数写进主 ViewModel 的时间。
    stats_at: std::sync::Mutex<Option<Instant>>,
    /// 合成的后台状态：App 在后台且没有网页媒体在放。为真时不探测、不自动重连。
    background: watch::Sender<bool>,
    /// App 自己报的可见性。
    app_backgrounded: AtomicBool,
    /// 插件网页的音频在放：退到后台也保持探测和自动重连。
    page_media: AtomicBool,
    /// 收新消息的方式是保持连接（ADR-0003）：壳的前台服务让进程和隧道在后台活着，核心不停探测和自动重连。
    keep_connection: AtomicBool,
    /// 媒体停下后延迟释放保活的任务，重新播放时取消。
    media_release: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// 每次媒体状态变化加一。释放任务醒来后只认自己那次，被后来的播放抢先时不覆盖。
    media_generation: AtomicU64,
    /// 定时检查开着。只有测试会关：要证明重连是某个动作促成的，不是碰巧赶上下一次检查。
    periodic_checks: watch::Sender<bool>,
    /// 每来一次网络变化加一，合并连着来的回调。
    network_changes: AtomicU64,
    /// 服务端现在的地址还没取到：最早一次失败起为真，取到就清掉，断开期间保持。
    addresses_stale: AtomicBool,
    /// 当前取地址的任务。重连时取消旧的，同一时刻只跑一个。
    address_refresh: std::sync::Mutex<Option<JoinHandle<()>>>,
    /// 第几次配对。旧一次的看门线程和迟到的结果只认自己那次，不盖掉后来的配对。
    pair_attempt: AtomicU64,
    /// 正在进行的配对从什么时候开始、到了哪一步；进度心跳从这里反映进 ViewModel。
    pairing_progress: std::sync::Mutex<Option<pairing::PairingProgress>>,
    /// 上次整个重建 endpoint 的时间。触发点照它限流，网络真断时不跟着失败的检查连环重建。
    endpoint_rebuilt_at: std::sync::Mutex<Option<Instant>>,
    /// 正在跑的配对任务。取消时中止它：被放弃的配对不该在后台继续连到自己的时限。
    pairing_task: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    #[cfg(feature = "test-hooks")]
    pub(crate) test_hooks: bind::TestHooks,
}

impl HomeLink {
    /// 读取设备密钥；上次连过服务器时自动重连。
    pub(crate) fn new(
        runtime: Handle,
        hub: Arc<wonton_tunnel::sharing::ProxyHub>,
        storage: Storage,
        network: Network,
        view: Option<watch::Sender<HomeViewModel>>,
        push_method: watch::Receiver<PushMethod>,
    ) -> Result<Arc<Self>, CoreError> {
        let device_key = storage.device_key()?;
        tracing::info!(device_id = %device_key.public(), "device identity of this phone");
        let saved = storage.ticket()?;
        let view = reuse(
            view,
            HomeViewModel {
                device_id: device_key.public().to_string(),
                server_id: None,
                status: HomeStatus::NotConfigured,
                path: None,
                notice: None,
                details: ConnectionDetails::default(),
                proxy: None,
                web_url: None,
                pairing: None,
                invite: None,
                keep_connection: false,
            },
        );
        let home = Arc::new(Self {
            runtime,
            hub,
            storage,
            device_key,
            network,
            view,
            ticket: std::sync::Mutex::new(None),
            core_url: std::sync::Mutex::new(None),
            link: Mutex::new(None),
            lifecycle_client: std::sync::Mutex::new(None),
            tracker: std::sync::Mutex::default(),
            quality: std::sync::Mutex::default(),
            recount: watch::Sender::new(()),
            stats_at: std::sync::Mutex::new(None),
            background: watch::Sender::new(false),
            app_backgrounded: AtomicBool::new(false),
            page_media: AtomicBool::new(false),
            keep_connection: AtomicBool::new(false),
            media_release: std::sync::Mutex::new(None),
            media_generation: AtomicU64::new(0),
            periodic_checks: watch::Sender::new(true),
            network_changes: AtomicU64::new(0),
            addresses_stale: AtomicBool::new(false),
            address_refresh: std::sync::Mutex::default(),
            pair_attempt: AtomicU64::new(0),
            pairing_progress: std::sync::Mutex::new(None),
            endpoint_rebuilt_at: std::sync::Mutex::new(None),
            pairing_task: std::sync::Mutex::new(None),
            #[cfg(feature = "test-hooks")]
            test_hooks: bind::TestHooks::default(),
        });
        home.runtime.spawn(background::follow_keep_connection(
            Arc::downgrade(&home),
            home.subscribe(),
            push_method,
        ));
        if let Some(text) = saved {
            // 只有校验过的连接信息才会被保存，读出来不合法说明存储坏了。
            let ticket = Ticket::parse(&text).map_err(|reason| CoreError::Storage {
                reason: l10n!(CATALOG, language(), "home-ticket-corrupt", reason = reason),
            })?;
            home.start(text, ticket);
        }
        Ok(home)
    }

    pub(crate) fn view(&self) -> HomeViewModel {
        self.view.borrow().clone()
    }

    /// 换服务器时新作用域接着用这个发送端，壳的订阅不断（ADR-0042）。
    pub(crate) fn sender(&self) -> watch::Sender<HomeViewModel> {
        self.view.clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<HomeViewModel> {
        self.view.subscribe()
    }

    /// `ConnectHome`：校验并保存连接信息，然后在后台建连。和当前用的连接信息相同时什么都不做：
    /// 连接断了会自动重连，重建反而会让正在用本地代理的 WebView 断开。
    pub(crate) fn connect(self: &Arc<Self>, text: &str) -> Result<(), CoreError> {
        let ticket = Ticket::parse(text).map_err(|reason| CoreError::InvalidInput { reason })?;
        if self.lock_ticket().as_deref() == Some(text) {
            return Ok(());
        }
        self.storage.save_ticket(text)?;
        self.start(text.to_owned(), ticket);
        Ok(())
    }

    /// `NetworkChanged`：让 iroh 重新检查路径，取消旧网络上的建连，确认现有连接还通。在后台时只做第一件，回到前台再确认。
    pub(crate) fn network_changed(self: &Arc<Self>, network: DeviceNetwork) {
        self.lock_tracker().network_changed(network, Instant::now());
        self.recount();
        let change = self.network_changes.fetch_add(1, Ordering::SeqCst) + 1;
        let home = self.clone();
        self.runtime.spawn(async move {
            tokio::time::sleep(NETWORK_CHANGE_SETTLE).await;
            if home.network_changes.load(Ordering::SeqCst) != change {
                return;
            }
            let Some((endpoint, client)) = home.current_link().await else {
                return;
            };
            endpoint.network_change().await;
            if *home.background.borrow() {
                return;
            }
            let has_network = network != DeviceNetwork::None;
            if has_network {
                client.retry_now();
            }
            let redials = async {
                for wait in NETWORK_CHANGE_REDIALS.into_iter().filter(|_| has_network) {
                    tokio::time::sleep(wait).await;
                    let superseded = home.network_changes.load(Ordering::SeqCst) != change;
                    if superseded || client.status() == TunnelStatus::Connected {
                        return;
                    }
                    client.retry_now();
                }
            };
            let (checked, ()) = tokio::join!(client.probe(NETWORK_CHANGE_DEADLINE), redials);
            if let Err(err) = checked {
                tracing::info!("tunnel check after a network change failed: {err}");
                // 探测只断开不重拨，马上换新连接，不等下一次定时检查。
                if has_network && !*home.background.borrow() {
                    home.check(&client).await;
                }
            }
            home.recount();
        });
    }

    /// `RetryConnection`：清零退避，马上确认连接；没回应时限时结束进度，自动重连继续。
    pub(crate) fn retry(self: &Arc<Self>) -> Result<(), CoreError> {
        if self.lock_ticket().is_none() {
            return Err(CoreError::NotReady {
                reason: l10n!(CATALOG, language(), "home-no-connection-info"),
            });
        }
        self.lock_tracker().retry_started(Instant::now());
        self.recount();
        let home = self.clone();
        self.runtime.spawn(async move {
            if let Some((endpoint, client)) = home.current_link().await {
                endpoint.network_change().await;
                client.retry_now();
                match tokio::time::timeout(LONG_OUTAGE, client.revalidate()).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(err)) => tracing::info!("retry failed: {err}"),
                    Err(_) => tracing::info!(
                        "retry confirmation timed out; automatic reconnect continues"
                    ),
                }
            }
            home.lock_tracker().retry_finished();
            home.recount();
        });
        Ok(())
    }

    /// 主人手动刷新时，被拒绝过就再拨一次：这台设备可能刚被加回去。其他原因的断开本来就在自己重试。
    pub(crate) async fn retry_if_rejected(&self) {
        if let Some((_, client)) = self.current_link().await
            && client.status() == TunnelStatus::Disconnected(TunnelError::Unauthorized)
        {
            client.retry_now();
        }
    }

    /// 聊天正在补上断开期间的消息。追赶指示流同值也算一次变化，没变就不重算。
    pub(crate) fn set_catching_up(&self, catching_up: bool) {
        if self.lock_tracker().set_catching_up(catching_up) {
            self.recount();
        }
    }

    /// 当前的隧道客户端；还没有连接信息时为空。
    pub(crate) async fn tunnel(&self) -> Option<TunnelClient> {
        self.current_link().await.map(|(_, client)| client)
    }

    async fn current_link(&self) -> Option<(Endpoint, TunnelClient)> {
        let current = self.link.lock().await;
        current
            .as_ref()
            .map(|link| (link.endpoint.clone(), link.client.clone()))
    }

    /// 挂起检测的布防跟着 App 的可见性走（`AppBackgrounded`），不看合成的后台状态：保持连接（ADR-0003）靠
    /// 前台服务减少进程被冻结的可能，但不豁免，冻结醒来也不会有 `AppForegrounded`，计时是唯一的检测手段。
    /// 回前台先保留检测，resume 检查完成后才关，避免网页抢先开流。
    fn arm_suspend_gate(&self) {
        let client = self.lock_lifecycle_client();
        if let Some(client) = client.as_ref() {
            // dispatch 也会从原生壳的非运行时线程调用。
            let _runtime = self.runtime.enter();
            client.set_suspendable(true);
        }
    }

    fn lock_lifecycle_client(&self) -> std::sync::MutexGuard<'_, Option<TunnelClient>> {
        self.lifecycle_client
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn recount(&self) {
        self.recount.send_replace(());
    }

    fn lock_tracker(&self) -> std::sync::MutexGuard<'_, Tracker> {
        self.tracker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_quality(&self) -> std::sync::MutexGuard<'_, Quality> {
        self.quality
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_ticket(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.ticket
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_stats_at(&self) -> std::sync::MutexGuard<'_, Option<Instant>> {
        self.stats_at
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

    /// 配对时填的设备名；`ConnectHome` 给的连接信息里没有，存储坏了也为空。
    pub(crate) fn device_name(&self) -> Option<String> {
        let ticket = Ticket::parse(self.lock_ticket().as_deref()?).ok()?;
        ticket.device_name().map(str::to_owned)
    }

    fn start(self: &Arc<Self>, text: String, ticket: Ticket) {
        *self.lock_ticket() = Some(text);
        *self
            .core_url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(ticket.core_url());
        self.lock_tracker().new_server();
        self.view.send_modify(|view| {
            view.server_id = Some(ticket.server_id().to_string());
            view.web_url = ticket.web_url().map(str::to_owned);
            view.status = HomeStatus::Connecting;
            view.path = None;
            view.proxy = None;
            view.notice = None;
        });
        self.runtime.spawn(self.clone().establish(ticket));
    }

    async fn establish(self: Arc<Self>, ticket: Ticket) {
        let mut current = self.link.lock().await;
        if let Some(old) = current.take() {
            *self.lock_lifecycle_client() = None;
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
                view.details.last_failure = Some(message.clone());
                view.notice = Some(ConnectionNotice::Unreachable { retrying: false });
                view.status = HomeStatus::Disconnected {
                    reason: HomeFailure::TransportUnavailable,
                    message,
                };
            }),
        }
    }

    async fn open(self: &Arc<Self>, ticket: &Ticket) -> Result<Link, String> {
        let endpoint = self.bind_endpoint(ticket.relays(), 1, language()).await?;
        // 只建最优先的一档（ADR-0048）；掉档回退由这个任务盯着家中继状态做。DirectOnly 不连中继，
        // 连接信息里一个中继都没有时没有表，也不回退。
        if self.network == Network::Internet && !ticket.relays().is_empty() {
            spawn_tier_fallback(
                endpoint.clone(),
                RelayTable::from_relays(ticket.relays().to_vec()),
                FALLBACK_AFTER,
                None,
            );
        }
        let client = TunnelClient::new(endpoint.clone(), ticket.server_addr());
        let credential = ProxyCredential::generate();
        let domain = ticket.base_domain().to_owned();
        let access = ProxyAccess::Credential {
            credential: credential.clone(),
            domain: domain.clone(),
        };
        let proxy = LocalProxy::bind(client.clone(), (Ipv4Addr::LOCALHOST, 0).into(), access)
            .await
            .map_err(|e| {
                l10n!(
                    CATALOG,
                    language(),
                    "home-proxy-start-failed",
                    error = e.to_string()
                )
            })?;
        let registration = self
            .hub
            .register(proxy.local_addr(), credential, domain.clone())
            .map_err(|_| "proxy route registration failed".to_owned())?;
        let credential = registration.credential();
        {
            let mut current = self.lock_lifecycle_client();
            client.set_suspendable(self.app_backgrounded.load(Ordering::SeqCst));
            *current = Some(client.clone());
        }
        let settings = ProxySettings {
            host: Ipv4Addr::LOCALHOST.to_string(),
            port: registration.local_addr().port(),
            username: credential.username().to_owned(),
            password: credential.password().to_owned(),
            domain,
        };
        self.view
            .send_modify(|view| view.proxy = Some(settings.clone()));
        let watcher = self.runtime.spawn(refresh::follow_connection(
            Arc::downgrade(self),
            endpoint.clone(),
            client.clone(),
            settings,
        ));
        let checker = self.runtime.spawn(keep_checking(
            Arc::downgrade(self),
            endpoint.clone(),
            client.clone(),
        ));
        Ok(Link {
            endpoint,
            client,
            proxy,
            registration,
            watcher,
            checker,
        })
    }

    async fn resume(self: Arc<Self>) {
        let started = Instant::now();
        let Some((endpoint, client)) = self.current_link().await else {
            return;
        };
        tracing::info!(elapsed = ?started.elapsed(), "STEP link");
        // 挂起期间网络可能变了（换 Wi-Fi、切蜂窝），让 iroh 重新检查网卡和路径。
        endpoint.network_change().await;
        tracing::info!(elapsed = ?started.elapsed(), "STEP network_change");
        client.retry_now();
        tracing::info!(elapsed = ?started.elapsed(), "STEP retry_now");
        // 挂起久了，服务端已经放弃这条连接，手机却收不到关闭帧：限时确认，没有回应就立刻换新连接，
        // 不等 30 秒的空闲超时（见 ADR-0004 的真机实测）。
        if let Err(err) = client.revalidate().await {
            tracing::info!("tunnel check failed: {err}");
        }
        tracing::info!(elapsed = ?started.elapsed(), "STEP revalidate");
        // 这一趟可能撞上刚结束、已经失败的另一次检查（回来的确认、挂起检测自己那次），那样它立刻返回、连接还是
        // 死的：连接不是好的就再拨一次，别让它拖到空闲超时。
        if client.status() != TunnelStatus::Connected
            && let Err(err) = client.ping().await
        {
            tracing::info!("tunnel redial failed: {err}");
        }
        tracing::info!(elapsed = ?started.elapsed(), "STEP redial");
        {
            let current = self.lock_lifecycle_client();
            if let Some(client) = current.as_ref() {
                // 检查期间可能再次进后台或更换 Link，不能用本次 resume 开始时的状态覆盖它。
                client.set_suspendable(self.app_backgrounded.load(Ordering::SeqCst));
            }
        }
        self.recount();
        tracing::info!(
            elapsed = ?started.elapsed(),
            status = %client.status(),
            "checked the tunnel after coming back to the foreground"
        );
        // 后台时停下过取地址：地址还是过时就接着取。
        if self.addresses_stale.load(Ordering::Relaxed) && self.connected().await {
            self.spawn_address_refresh();
        }
    }

    /// 和服务端做一次往返（必要时重连）。状态由 `follow_connection` 跟着隧道客户端更新。
    async fn check(&self, client: &TunnelClient) {
        if let Err(err) = client.ping().await {
            tracing::info!("tunnel check failed: {err}");
        }
        self.recount();
    }

    /// 把隧道的情况算成 ViewModel，有变化才推送。视图已归别的连接时返回 [`Published::Superseded`]，
    /// 调用方该退出了；否则返回多久之后要重新算一次。
    fn publish(
        &self,
        tunnel: &TunnelStatus,
        endpoint: &Endpoint,
        client: &TunnelClient,
        proxy: &ProxySettings,
    ) -> Published {
        let now = Instant::now();
        let status = status_view(tunnel);
        let connected = status == HomeStatus::Connected;
        let path = client.path();
        let quality = {
            let mut quality = self.lock_quality();
            match path.as_ref() {
                Some(path) if connected => quality.observe(now, path),
                _ => quality.reset(),
            }
        };
        let path = path.map(path_view);
        let relay = relay_check(endpoint);
        let stats = client.stats();
        let mut tracker = self.lock_tracker();
        if let Some(recovery) = tracker.observe(&status, relay, stats.failed_dials, now) {
            let outage_ms = recovery.outage.as_millis();
            tracing::info!(outage_ms, trigger = recovery.trigger, "connection restored");
        }
        let notice = tracker.notice(&status, now);
        let details = ConnectionDetails {
            network: tracker.network(),
            relay: tracker.relay(),
            server: tracker.server_check(&status, now),
            last_connected_at_ms: stats.last_confirmed.and_then(unix_ms),
            handshake_ms: stats
                .handshake
                .map(|took| u32::try_from(took.as_millis()).unwrap_or(u32::MAX)),
            disconnects: stats.disconnects,
            failed_dials: stats.failed_dials,
            sent_packets: stats.sent_packets,
            lost_packets: stats.lost_packets,
            quality: quality.quality,
            loss_per_mille: quality.loss_per_mille,
            rtt_ms: quality.rtt_ms,
            last_failure: stats.last_failure.as_ref().map(ToString::to_string),
            addresses_stale: self.addresses_stale.load(Ordering::Relaxed),
            udp_flows: client.udp_flows(),
            udp_buffer_evicted_datagrams: client.udp_buffer_evicted(),
        };
        let tick = tracker.next_tick(connected);
        drop(tracker);
        let stats_due = connected && self.stats_due(now);
        let mut withheld = false;
        // 视图里的本地代理不是这条连接的，说明连接信息已被换过（换了服务器或重连），视图归新的
        // 连接管：这里再写会把 start 刚写下的 Connecting 盖回 Connected，而新连接的 proxy 还没进
        // 视图，App 会以为已经连上，这期间发的请求全部按“连不上”失败（#826）。判据放在
        // send_if_modified 的闭包里，和 start 的视图写同锁，中间插不进别的写。
        let superseded = std::cell::Cell::new(false);
        let changed = self.view.send_if_modified(|view| {
            if view.proxy.as_ref() != Some(proxy) {
                superseded.set(true);
                return false;
            }
            let mut details = details;
            let mut path = path;
            if connected && !stats_due {
                withheld |= details.sent_packets != view.details.sent_packets
                    || details.lost_packets != view.details.lost_packets
                    || details.last_connected_at_ms != view.details.last_connected_at_ms
                    || details.udp_flows != view.details.udp_flows
                    || details.udp_buffer_evicted_datagrams
                        != view.details.udp_buffer_evicted_datagrams
                    || details.loss_per_mille != view.details.loss_per_mille
                    || details.rtt_ms != view.details.rtt_ms;
                details.sent_packets = view.details.sent_packets;
                details.lost_packets = view.details.lost_packets;
                details.last_connected_at_ms = view.details.last_connected_at_ms;
                details.udp_flows = view.details.udp_flows.clone();
                details.udp_buffer_evicted_datagrams = view.details.udp_buffer_evicted_datagrams;
                details.loss_per_mille = view.details.loss_per_mille;
                details.rtt_ms = view.details.rtt_ms;
                if let (Some(new), Some(old)) = (&mut path, &view.path) {
                    withheld |= new.rtt_ms != old.rtt_ms;
                    new.rtt_ms = old.rtt_ms;
                }
            }
            let changed = view.status != status
                || view.path != path
                || view.notice != notice
                || view.details != details;
            view.status = status;
            view.path = path;
            view.notice = notice;
            view.details = details;
            changed
        });
        if superseded.get() {
            return Published::Superseded;
        }
        if changed && stats_due {
            *self.lock_stats_at() = Some(now);
        }
        // 有被压住的样本就到点再算一次，连接页每秒最多拿到一次新数字。
        let stats_tick = withheld.then(|| match *self.lock_stats_at() {
            Some(at) => STATS_INTERVAL.saturating_sub(now.duration_since(at)),
            None => STATS_INTERVAL,
        });
        Published::Tick(match (tick, stats_tick) {
            (Some(tick), Some(stats)) => Some(tick.min(stats)),
            (tick, stats) => tick.or(stats),
        })
    }

    /// 距上次把每包变化的计数写进主 ViewModel 过了多久；还没推过就是到期。
    fn stats_due(&self, now: Instant) -> bool {
        match *self.lock_stats_at() {
            Some(at) => now.duration_since(at) >= STATS_INTERVAL,
            None => true,
        }
    }
}

/// 一次 [`HomeLink::publish`] 的结果：视图已归别的连接（这个 watcher 该退出了），或者照常发布
/// 完，返回多久之后要重新算一次。
enum Published {
    Superseded,
    Tick(Option<Duration>),
}

/// 手机连不连得上中继：连得上说明手机能上网，这时家里没有回应，多半是家里的问题。没配中继时无从判断。
fn relay_check(endpoint: &Endpoint) -> ConnectionCheck {
    let relays = endpoint.home_relay_status().get();
    if relays.is_empty() {
        ConnectionCheck::Unknown
    } else if relays.iter().any(iroh::endpoint::RelayStatus::is_connected) {
        ConnectionCheck::Ok
    } else {
        ConnectionCheck::Failing
    }
}

fn unix_ms(time: std::time::SystemTime) -> Option<u64> {
    let since = time.duration_since(std::time::UNIX_EPOCH).ok()?;
    u64::try_from(since.as_millis()).ok()
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
            message: l10n!(CATALOG, language(), "home-tunnel-closed"),
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
        lan: path.is_lan(),
        rtt_ms: u32::try_from(path.rtt.as_millis()).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests;
