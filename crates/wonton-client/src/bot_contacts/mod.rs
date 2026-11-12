//! Bot 资料页的“联系人”（ADR-0045、ADR-0050）：一次看一个 Bot，经隧道取自家核心的联系人接口。
//!
//! [`Action::RefreshBotContacts`] 选定一个 Bot；之后对任何 Bot 的操作（包括在“等你确认”里同意别的 Bot 的请求）
//! 都把 ViewModel 切到那个 Bot，结果和失败原因都落在这里。有一行在等对方（Bot 等它的归属人同意，人等他接受）
//! 时隔几秒取一次列表，跟上对方的处理；都没有了就停。

mod api;

use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::watch::Sender as WatchSender;
use tokio::sync::{mpsc, watch};
use wonton_l10n::l10n;
use wonton_protocol::{
    Action, BotContactsState, BotContactsViewModel, BotRef, ContactError, ContactKind,
    ContactRequest, ContactState, CoreError, HomeStatus,
};

use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};
use crate::scope::reuse;
use api::{Failure, Op};

/// 有一行在等对方时隔这么久取一次列表：对方是人手点的同意或接受，几秒内跟上就够。核心查人的邀请结果是
/// 取列表时顺带查的，所以这里也决定了“对方接受了”多快显示出来。
const POLL_INTERVAL: Duration = Duration::from_secs(5);

pub(crate) struct BotContactsLink {
    handle: Handle,
    home: Arc<HomeLink>,
    view: WatchSender<BotContactsViewModel>,
    languages: std::sync::Mutex<Vec<String>>,
    client: std::sync::Mutex<Option<(wonton_protocol::ProxySettings, reqwest::Client)>>,
    /// 请求按动作的先后一个一个发：连着点了拒绝和删除，后点的结果最后落地，ViewModel 不会停在先点的那份上。
    jobs: mpsc::UnboundedSender<Job>,
}

struct Job {
    bot: BotRef,
    op: Op,
    /// 壳点的动作失败要报；后台重取失败不盖掉正显示的列表。
    report: bool,
}

impl BotContactsLink {
    pub(crate) fn new(
        handle: Handle,
        home: Arc<HomeLink>,
        view: Option<WatchSender<BotContactsViewModel>>,
    ) -> Arc<Self> {
        let initial = BotContactsViewModel {
            bot: None,
            state: BotContactsState::Ready,
            contacts: Vec::new(),
        };
        let (jobs, queued) = mpsc::unbounded_channel();
        let link = Arc::new(Self {
            handle,
            home,
            view: reuse(view, initial),
            languages: std::sync::Mutex::new(Vec::new()),
            client: std::sync::Mutex::new(None),
            jobs,
        });
        link.handle.spawn(work(Arc::downgrade(&link), queued));
        link.handle.spawn(follow_home(Arc::downgrade(&link)));
        link.handle.spawn(follow_waiting(Arc::downgrade(&link)));
        link
    }

    pub(crate) fn sender(&self) -> WatchSender<BotContactsViewModel> {
        self.view.clone()
    }

    pub(crate) fn dispatch(self: &Arc<Self>, action: Action) -> Result<(), CoreError> {
        let (bot, op) = match action {
            Action::RefreshBotContacts { bot } => (bot, Op::List),
            Action::RequestBotContact { bot, user_id } => (bot, request(user_id, ContactKind::Bot)),
            Action::RequestPersonContact { bot, user_id } => {
                (bot, request(user_id, ContactKind::Person))
            }
            Action::ApproveBotContact { bot, user_id } => (bot, Op::Approve(user_id)),
            Action::RejectBotContact { bot, user_id } => (bot, Op::Reject(user_id)),
            Action::RemoveBotContact { bot, user_id } => (bot, Op::Remove(user_id)),
            _ => {
                return Err(CoreError::InvalidInput {
                    reason: l10n!(CATALOG, language(), "contacts-invalid-action"),
                });
            }
        };
        self.select(&bot);
        self.enqueue(bot, op, true);
        Ok(())
    }

    /// 壳告诉核心 App 的语言。失败说明是家里的服务器按旧语言写的，换了语言就重新取。
    pub(crate) fn set_languages(self: &Arc<Self>, languages: Vec<String>) {
        {
            let mut current = lock(&self.languages);
            if *current == languages {
                return;
            }
            *current = languages;
        }
        if let Some(bot) = self.viewed() {
            self.enqueue(bot, Op::List, false);
        }
    }

    /// 切到这个 Bot：换了 Bot 时清掉上一个的列表，拨成“正在取”。
    fn select(&self, bot: &BotRef) {
        self.view.send_modify(|view| {
            if view.bot.as_ref() != Some(bot) {
                view.bot = Some(bot.clone());
                view.contacts.clear();
            }
            view.state = BotContactsState::Loading;
        });
    }

    /// 做一件事。结果只在 ViewModel 还看着这个 Bot 时落地：期间壳切到了别的 Bot，就不拿旧 Bot 的结果盖掉。
    async fn run(&self, Job { bot, op, report }: Job) {
        let outcome = match self.api() {
            Ok(api) => api.run(&bot, &op).await,
            Err(failure) => Err(failure),
        };
        self.view.send_modify(|view| {
            if view.bot.as_ref() != Some(&bot) {
                return;
            }
            match outcome {
                Ok(list) => {
                    view.contacts = list.contacts;
                    view.state = BotContactsState::Ready;
                }
                Err(failure) if report || view.state != BotContactsState::Ready => {
                    tracing::info!(
                        error = failure.error.as_str(),
                        "bot contacts operation failed"
                    );
                    view.state = BotContactsState::Failed {
                        error: failure.error,
                        message: failure.message,
                    };
                }
                // 后台轮询失败不盖掉正显示的列表，下次照样取。
                Err(_) => {}
            }
        });
    }

    fn api(&self) -> Result<api::Api, Failure> {
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
        Ok(api::Api::new(base, client, &lock(&self.languages)))
    }

    fn viewed(&self) -> Option<BotRef> {
        self.view.borrow().bot.clone()
    }

    fn enqueue(&self, bot: BotRef, op: Op, report: bool) {
        // 只在运行时关停、工作任务已经没了时发不出去，那时也没有界面等结果。
        let _ = self.jobs.send(Job { bot, op, report });
    }
}

async fn work(link: Weak<BotContactsLink>, mut queued: mpsc::UnboundedReceiver<Job>) {
    while let Some(job) = queued.recv().await {
        let Some(link) = link.upgrade() else { return };
        link.run(job).await;
    }
}

fn request(user_id: String, kind: ContactKind) -> Op {
    Op::Request(ContactRequest { user_id, kind })
}

fn not_connected() -> Failure {
    Failure::new(
        ContactError::TransportUnavailable,
        l10n!(CATALOG, language(), "peers-not-connected"),
    )
}

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 有一行在等对方。
fn waiting(view: &BotContactsViewModel) -> bool {
    view.contacts
        .iter()
        .any(|contact| contact.state == ContactState::Pending)
}

async fn follow_waiting(link: Weak<BotContactsLink>) {
    let Some(strong) = link.upgrade() else { return };
    let mut view: watch::Receiver<BotContactsViewModel> = strong.view.subscribe();
    drop(strong);
    loop {
        if !waiting(&view.borrow_and_update()) {
            if view.changed().await.is_err() {
                return;
            }
            continue;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
        let Some(link) = link.upgrade() else { return };
        if !waiting(&link.view.borrow()) {
            continue;
        }
        if let Some(bot) = link.viewed() {
            link.enqueue(bot, Op::List, false);
        }
    }
}

/// 连上家里的服务器（启动、断线恢复）时，正看着的 Bot 重新取一次。
async fn follow_home(link: Weak<BotContactsLink>) {
    let Some(mut home) = link.upgrade().map(|link| link.home.subscribe()) else {
        return;
    };
    let mut connected = false;
    loop {
        let now_connected = home.borrow_and_update().status == HomeStatus::Connected;
        let Some(strong) = link.upgrade() else { return };
        if now_connected
            && !connected
            && let Some(bot) = strong.viewed()
        {
            strong.enqueue(bot, Op::List, false);
        }
        connected = now_connected;
        drop(strong);
        if home.changed().await.is_err() {
            return;
        }
    }
}
