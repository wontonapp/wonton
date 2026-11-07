//! “其他服务器”页面（ADR-0050）：配对过的服务器列表和主人的出码，经隧道取自家核心的管理接口。
//! 每台服务器一个作用域（ADR-0042），作用域里的这份状态就属于这一台。
//!
//! 状态 [`PeersState`]：取列表时 `Loading`，最近一次取到了 `Ready`，最近一次操作（取列表、出码、扫码、
//! 删除）失败 `Failed`。连上家里的服务器时取一次列表——设置首页的“其他服务器”一项要显示已配对的台数，
//! 条目只在取到过列表时给（核心只让主人设备读这个接口，非主人的设备取不到，也就看不到这一项）。
//! 扫码、删除成功后再取一次；出的码显示到过期，过期后清掉。有东西在等对方的时候——出的码还没作废，
//! 列表里有还没批的请求，或有一行刚配对还没探活出结果——再隔几秒取一次，跟上对方的扫码和同意，
//! 不用退出页面再进；都没有了就停。列表取到过（包括空的）以后按核心的探活周期取，已配对服务器的
//! 连上、掉线、对方删了配对在半分钟内显示出来，设置首页的台数也跟着变。

mod api;

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::sync::watch::Sender as WatchSender;
use wonton_l10n::l10n;
use wonton_protocol::{
    Action, CoreError, HomeStatus, PeerCode, PeerError, PeerStatus, PeersState, PeersViewModel,
    SecretString,
};

use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};
use crate::scope::reuse;

/// 有东西在等对方时隔这么久取一次列表。和插件页在安装、升级、卸载时重新取已安装列表的节奏一致：
/// 同意配对是对方主人手点的动作，几秒内跟上就够；再快是白费的请求，再慢主人盯着“等对方同意”等得久。
const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// 没有可等的东西以后取列表的间隔：核心每 30 秒探活一次已配对的服务器（`FEDERATION_PING_INTERVAL`），
/// 按同样的周期取就够——对方掉线、对方删了配对，以及别的设备上的配对和删除，都在半分钟左右跟上，
/// 不用退出页面再进。更快只是白费的请求，探活本身比这慢。
const STATUS_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) struct PeersLink {
    handle: Handle,
    home: Arc<HomeLink>,
    view: WatchSender<PeersViewModel>,
    /// 请求带的语言（Accept-Language），壳用 `SetLanguages` 告诉核心。
    languages: std::sync::Mutex<Vec<String>>,
    /// 本地代理的凭据和端口随每次建连换；同一个代理上复用连接。
    client: std::sync::Mutex<Option<(wonton_protocol::ProxySettings, reqwest::Client)>>,
    /// 同一时刻只发一个列表请求：扫码、删除成功后的刷新排在正在跑的后面，取到的是最新数据。
    list_queue: tokio::sync::Mutex<()>,
    /// 取到过列表：只有在这之后状态才算“列表是最新的”，设置首页才给那一项。
    listed: AtomicBool,
    /// 出过的第几张码。过期的定时器醒来时码已经换了（新出一张、刷新）就不要清。
    code_epoch: AtomicU64,
}

impl PeersLink {
    pub(crate) fn new(
        handle: Handle,
        home: Arc<HomeLink>,
        view: Option<WatchSender<PeersViewModel>>,
    ) -> Arc<Self> {
        let initial = PeersViewModel {
            state: PeersState::Loading,
            peers: Vec::new(),
            requests: Vec::new(),
            rejected: Vec::new(),
            code: None,
        };
        let link = Arc::new(Self {
            handle,
            home,
            view: reuse(view, initial),
            languages: std::sync::Mutex::new(Vec::new()),
            client: std::sync::Mutex::new(None),
            list_queue: tokio::sync::Mutex::new(()),
            listed: AtomicBool::new(false),
            code_epoch: AtomicU64::new(0),
        });
        link.spawn(follow_home(Arc::downgrade(&link)));
        link.spawn(follow_waiting(Arc::downgrade(&link)));
        link.spawn(follow_status(Arc::downgrade(&link)));
        link
    }

    /// 换服务器时新作用域接着用这个发送端，壳的订阅不断（ADR-0042）。
    pub(crate) fn sender(&self) -> WatchSender<PeersViewModel> {
        self.view.clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<PeersViewModel> {
        self.view.subscribe()
    }

    pub(crate) fn dispatch(self: &Arc<Self>, action: Action) -> Result<(), CoreError> {
        match action {
            Action::RefreshPeers => {
                self.refresh();
                Ok(())
            }
            Action::ShowPeerCode => {
                self.show_code();
                Ok(())
            }
            Action::ScanPeerCode { code } => {
                self.scan(code);
                Ok(())
            }
            Action::RemovePeer { server_name } => {
                self.remove(&server_name);
                Ok(())
            }
            _ => Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "peers-invalid-action"),
            }),
        }
    }

    /// 壳告诉核心 App 的语言。取到的列表里的“详情”是家里的服务器按旧语言写的，换了语言就重新取。
    pub(crate) fn set_languages(self: &Arc<Self>, languages: Vec<String>) {
        {
            let mut current = lock(&self.languages);
            if *current == languages {
                return;
            }
            *current = languages;
        }
        if self.listed.load(Ordering::SeqCst) {
            self.refresh();
        }
    }

    /// 取列表和出码状态。失败不影响已显示的列表，下次成功时一起换掉。
    fn refresh(self: &Arc<Self>) {
        let link = self.clone();
        self.spawn(async move { link.fetch(true).await });
    }

    /// 取列表和出码状态。失败不影响已显示的列表，下次成功时一起换掉。
    /// `mark_loading`：取之前先把状态拨成 `Loading`，主人的操作要马上有反馈；后台轮询不拨——页面上
    /// 那份列表还在显示，几秒一次的拨动没有信息，设置首页的台数只在 `Ready` 时给，会跟着一闪一闪。
    async fn fetch(self: Arc<Self>, mark_loading: bool) {
        // 排队而不是跳过：扫码、删除成功后的刷新要排在正在跑的请求后面，才能取到它们的结果。
        let _guard = self.list_queue.lock().await;
        if mark_loading {
            self.view
                .send_modify(|view| view.state = PeersState::Loading);
        }
        let outcome = match self.api() {
            Ok(api) => api.list().await,
            Err(failure) => Err(failure),
        };
        match outcome {
            Ok(list) => {
                self.listed.store(true, Ordering::SeqCst);
                self.set_code(list.code);
                self.view.send_modify(|view| {
                    view.peers = list.peers;
                    view.requests = list.requests;
                    view.rejected = list.rejected;
                    view.state = PeersState::Ready;
                });
            }
            Err(failure) => self.fail(failure),
        }
    }

    /// 出一张码。码显示到过期（`set_code` 里的定时器负责清）；列表还没取到过时状态不动，
    /// 等列表请求落地再变成“列表是最新的”。
    fn show_code(self: &Arc<Self>) {
        let link = self.clone();
        self.spawn(async move {
            let outcome = match link.api() {
                Ok(api) => api.show_code().await,
                Err(failure) => Err(failure),
            };
            match outcome {
                Ok(code) => {
                    if link.listed.load(Ordering::SeqCst) {
                        link.view.send_modify(|view| view.state = PeersState::Ready);
                    }
                    link.set_code(Some(code));
                }
                Err(failure) => link.fail(failure),
            }
        });
    }

    /// 把扫来的码交给自己的核心。成功后重新取列表：那条先记成“等对方同意”，对方点了同意才成立。
    fn scan(self: &Arc<Self>, code: SecretString) {
        let link = self.clone();
        self.spawn(async move {
            let outcome = match link.api() {
                Ok(api) => api.scan(&code).await,
                Err(failure) => Err(failure),
            };
            match outcome {
                Ok(()) => link.refresh(),
                Err(failure) => link.fail(failure),
            }
        });
    }

    /// 删掉和一台服务器的配对。成功后先把那一行从列表里去掉（设置首页的台数马上跟上，取列表失败也不回退），
    /// 再重新取列表：两边名单都撤了，已有的跨机房间留着。
    fn remove(self: &Arc<Self>, server_name: &str) {
        let link = self.clone();
        let server_name = server_name.to_owned();
        self.spawn(async move {
            let outcome = match link.api() {
                Ok(api) => api.remove(&server_name).await,
                Err(failure) => Err(failure),
            };
            match outcome {
                Ok(()) => {
                    link.view.send_modify(|view| {
                        view.peers.retain(|row| row.server_name != server_name)
                    });
                    link.refresh();
                }
                Err(failure) => link.fail(failure),
            }
        });
    }

    fn api(&self) -> Result<api::Api, api::Failure> {
        let proxy = self.home.view().proxy.ok_or_else(not_connected)?;
        let base = self.home.core_url().ok_or_else(not_connected)?;
        let client = {
            let mut cached = lock(&self.client);
            match cached.as_ref() {
                Some((settings, client)) if *settings == proxy => client.clone(),
                _ => {
                    let client = api::proxied_client(&proxy)?;
                    *cached = Some((proxy.clone(), client.clone()));
                    client
                }
            }
        };
        Ok(api::Api::new(base, client, lock(&self.languages).clone()))
    }

    /// 记下一次失败。列表请求正在路上时也一样：扫码、出码的结果比一份还没到的列表要紧。
    fn fail(&self, failure: api::Failure) {
        tracing::info!(error = %failure.error.as_str(), "peers operation failed");
        self.view.send_modify(|view| {
            view.state = PeersState::Failed {
                error: failure.error,
                message: failure.message,
            }
        });
    }

    /// 记下新的出码（或清掉），并安排到点清掉。和正显示的同一张码就不动：等待时的轮询每隔几秒取到它
    /// 一次，别每次都重摆过期定时器、往壳重推一遍。
    fn set_code(self: &Arc<Self>, code: Option<PeerCode>) {
        let unchanged = self.view.borrow().code.as_ref() == code.as_ref();
        if unchanged {
            return;
        }
        let epoch = self.code_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.view.send_modify(|view| view.code = code.clone());
        let Some(code) = code else { return };
        let link = self.clone();
        self.spawn(async move {
            let now = unix_ms();
            if code.expires_at_ms > now {
                tokio::time::sleep(Duration::from_millis(code.expires_at_ms - now)).await;
            }
            // 码在这期间被换掉了（新出一张、刷新拿到别的）就什么都不做。
            if link.code_epoch.load(Ordering::SeqCst) == epoch {
                link.view.send_modify(|view| view.code = None);
            }
        });
    }

    /// 在作用域的运行时里跑：作用域关闭（换服务器、核心释放）时任务跟着停。
    fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        self.handle.spawn(task);
    }
}

fn not_connected() -> api::Failure {
    api::Failure::new(
        PeerError::TransportUnavailable,
        l10n!(CATALOG, language(), "peers-not-connected"),
    )
}

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 有东西在等对方时自己取列表：出的码还没作废（对方随时可能扫掉），列表里有还没批的请求（对方随时
/// 可能点同意），或有一行刚配对、第一次探活还没出结果（几十秒内见分晓）。都没有了就睡到状态再变，
/// 这期间一个请求都不发；取列表失败不停轮询，状态照现有规则进 `Failed`，下一次照样取。作用域
/// 关掉时这个任务跟着停。
async fn follow_waiting(link: Weak<PeersLink>) {
    let Some(strong) = link.upgrade() else { return };
    let mut view = strong.view.subscribe();
    drop(strong);
    loop {
        if !waiting(&view.borrow_and_update()) {
            if view.changed().await.is_err() {
                return;
            }
            continue;
        }
        // 刚出现的可等的东西多半是刚落地的那次取列表带来的，先隔一个周期再取。
        tokio::time::sleep(POLL_INTERVAL).await;
        let Some(link) = link.upgrade() else { return };
        if !waiting(&link.view.borrow()) {
            continue;
        }
        link.fetch(false).await;
    }
}

/// 现在有没有东西在等对方：出的码还没作废，列表里有还没批的请求，或有一行刚配对、第一次探活还没出结果。
fn waiting(view: &PeersViewModel) -> bool {
    view.code.is_some()
        || view.peers.iter().any(|row| {
            matches!(
                row.status,
                PeerStatus::Checking | PeerStatus::AwaitingApproval
            )
        })
}

/// 要不要按探活周期取列表：连上了家里的服务器，而且列表取到过（空列表也算——别的设备上配对、
/// 删除时这里才知道；取失败过也取，过一会儿再试）。页面开着时不另行登记：设置首页的那一行也要
/// 跟着列表变，它一直订阅着，壳的页面订阅分不出“有没有人在看”。
async fn follow_status(link: Weak<PeersLink>) {
    let Some(strong) = link.upgrade() else { return };
    let mut view = strong.view.subscribe();
    let mut home = strong.home.subscribe();
    drop(strong);
    loop {
        let connected = home.borrow_and_update().status == HomeStatus::Connected;
        let listed = {
            let view = view.borrow_and_update();
            !view.peers.is_empty() || !matches!(view.state, PeersState::Loading)
        };
        if !(connected && listed) {
            tokio::select! {
                changed = view.changed() => if changed.is_err() { return },
                changed = home.changed() => if changed.is_err() { return },
            }
            continue;
        }
        tokio::time::sleep(STATUS_INTERVAL).await;
        let Some(link) = link.upgrade() else { return };
        // 这期间连不上或有人在等更急的东西（码、等对方同意）：`follow_home` 和 `follow_waiting`
        // 管着，不重复取。
        if link.home.view().status != HomeStatus::Connected || waiting(&link.view.borrow()) {
            continue;
        }
        link.fetch(false).await;
    }
}

/// 跟着家里的连接：连上（App 启动、重新配对、断线恢复）时取一次列表。
async fn follow_home(link: Weak<PeersLink>) {
    let Some(mut home) = link.upgrade().map(|link| link.home.subscribe()) else {
        return;
    };
    let mut connected = false;
    loop {
        let now_connected = home.borrow_and_update().status == HomeStatus::Connected;
        let Some(link) = link.upgrade() else { return };
        if now_connected && !connected {
            link.refresh();
        }
        connected = now_connected;
        drop(link);
        if home.changed().await.is_err() {
            return;
        }
    }
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}
