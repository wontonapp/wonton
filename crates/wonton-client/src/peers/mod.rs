//! “其他服务器”页面（ADR-0050）：配对过的服务器列表和主人的出码，经隧道取自家核心的管理接口。
//! 每台服务器一个作用域（ADR-0042），作用域里的这份状态就属于这一台。
//!
//! 状态 [`PeersState`]：取列表时 `Loading`，最近一次取到了 `Ready`，最近一次操作（取列表、出码、扫码、
//! 删除）失败 `Failed`。连上家里的服务器时取一次列表——设置首页的“其他服务器”一项要显示已配对的台数，
//! 条目只在取到过列表时给（核心只让主人设备读这个接口，非主人的设备取不到，也就看不到这一项）。
//! 扫码、删除成功后再取一次；出的码显示到过期，过期后清掉。

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
    Action, CoreError, HomeStatus, PeerCode, PeerError, PeersState, PeersViewModel, SecretString,
};

use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};
use crate::scope::reuse;

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
        self.spawn(async move { link.fetch().await });
    }

    async fn fetch(self: Arc<Self>) {
        // 排队而不是跳过：扫码、删除成功后的刷新要排在正在跑的请求后面，才能取到它们的结果。
        let _guard = self.list_queue.lock().await;
        self.view
            .send_modify(|view| view.state = PeersState::Loading);
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

    /// 删掉和一台服务器的配对。成功后重新取列表：两边名单都撤了，已有的跨机房间留着。
    fn remove(self: &Arc<Self>, server_name: &str) {
        let link = self.clone();
        let server_name = server_name.to_owned();
        self.spawn(async move {
            let outcome = match link.api() {
                Ok(api) => api.remove(&server_name).await,
                Err(failure) => Err(failure),
            };
            match outcome {
                Ok(()) => link.refresh(),
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

    /// 记下新的出码（或清掉），并安排到点清掉。
    fn set_code(self: &Arc<Self>, code: Option<PeerCode>) {
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
