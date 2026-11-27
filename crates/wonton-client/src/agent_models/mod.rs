//! 一个 Bot 的“模型”页（ADR-0129）：经隧道调自家核心的模型接入接口，核心再交给这个 Bot 的 Agent 插件。
//!
//! [`AgentModelsAction::OpenAgentModels`] 选定一个 Bot，之后的操作都对着它和点开的那家服务商。请求按动作的先后一个一个发；
//! 结果只在页面还看着同一个 Bot（和同一家）时落地。取目录失败换掉页面（`state`）；别的动作失败页面照旧，原因放在
//! `action_problem`。设备码登录由核心按插件给的间隔轮询，到批准、拒绝、过期为止；浏览器授权等壳交回回跳网址。

mod api;
mod jobs;
mod login;
mod view;

use std::sync::{Arc, Weak};

use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::sync::watch::Sender as WatchSender;
use wonton_l10n::l10n;
use wonton_protocol::{
    AgentModelsAction, AgentModelsState, AgentModelsViewModel, BotRef, CoreError, HomeStatus,
    ModelCatalog, ModelListState, ModelTestState, SelectModel,
};

use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};
use crate::scope::reuse;
use jobs::{Job, Op};

pub(crate) struct AgentModelsLink {
    handle: Handle,
    home: Arc<HomeLink>,
    view: WatchSender<AgentModelsViewModel>,
    languages: std::sync::Mutex<Vec<String>>,
    client: std::sync::Mutex<Option<(wonton_protocol::ProxySettings, reqwest::Client)>>,
    jobs: mpsc::UnboundedSender<Job>,
    local: std::sync::Mutex<Local>,
}

/// ViewModel 之外要记住的。都带着 Bot：壳随时会切到别的 Bot，读的时候核对一遍，不拿旧 Bot 的用。
#[derive(Default)]
struct Local {
    /// 整份目录：搜索时重新过滤，点开一家时取它的接法。
    catalog: Option<(BotRef, ModelCatalog)>,
    login: Option<Login>,
}

/// 正在进行的一次登录。会话号只在核心和插件之间有意义，不给壳。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Login {
    bot: BotRef,
    provider: String,
    session: String,
    redirect: bool,
    /// 浏览器授权网址里的 `state`：回跳带回来的必须是它，见 [`AgentModelsLink::finish_login`]。
    state: Option<String>,
}

impl AgentModelsLink {
    pub(crate) fn new(
        handle: Handle,
        home: Arc<HomeLink>,
        view: Option<WatchSender<AgentModelsViewModel>>,
    ) -> Arc<Self> {
        let (jobs, queued) = mpsc::unbounded_channel();
        let link = Arc::new(Self {
            handle,
            home,
            view: reuse(view, AgentModelsViewModel::default()),
            languages: std::sync::Mutex::new(Vec::new()),
            client: std::sync::Mutex::new(None),
            jobs,
            local: std::sync::Mutex::new(Local::default()),
        });
        link.handle.spawn(work(Arc::downgrade(&link), queued));
        link.handle.spawn(follow_home(Arc::downgrade(&link)));
        link
    }

    pub(crate) fn sender(&self) -> WatchSender<AgentModelsViewModel> {
        self.view.clone()
    }

    pub(crate) fn dispatch(self: &Arc<Self>, action: AgentModelsAction) -> Result<(), CoreError> {
        use AgentModelsAction as A;
        let bot = match &action {
            A::OpenAgentModels { bot } => bot.clone(),
            _ => self.viewed().ok_or_else(invalid)?,
        };
        self.view.send_modify(|view| view.action_problem = None);
        match action {
            A::OpenAgentModels { .. } => self.open(bot),
            A::SearchModelProviders { text } => self.search(text),
            A::OpenModelProvider { provider } => self.open_provider(&bot, &provider)?,
            A::CloseModelProvider => {
                self.drop_login();
                self.view.send_modify(|view| view.provider = None);
            }
            A::SaveModelKey { key, base_url } => {
                let provider = self.busy()?;
                self.enqueue(
                    bot,
                    Op::SaveKey {
                        provider,
                        key,
                        base_url,
                    },
                );
            }
            A::DisconnectModelProvider => {
                let provider = self.busy()?;
                self.enqueue(bot, Op::Disconnect { provider });
            }
            A::StartModelLogin => self.start_login(bot)?,
            A::FinishModelLogin { callback_url } => self.finish_login(bot, callback_url)?,
            A::CancelModelLogin => self.drop_login(),
            A::SelectModel { model } => {
                let provider = self.busy()?;
                let select = SelectModel {
                    provider,
                    model,
                    confirmed: false,
                };
                self.enqueue(bot, Op::Select(select));
            }
            A::ConfirmModelSelection => self.confirm(bot)?,
            A::DismissModelConfirmation => self.view.send_modify(|view| {
                if let Some(page) = view.provider.as_mut() {
                    page.confirm = None;
                }
            }),
            A::TestModel => {
                self.view
                    .send_modify(|view| view.test = Some(ModelTestState::Running));
                self.enqueue(bot, Op::Test);
            }
        }
        Ok(())
    }

    /// 壳告诉核心 App 的语言。目录里的名字和说明是插件按语言写的，换了语言就重新取。
    pub(crate) fn set_languages(self: &Arc<Self>, languages: Vec<String>) {
        {
            let mut current = lock(&self.languages);
            if *current == languages {
                return;
            }
            *current = languages;
        }
        if let Some(bot) = self.viewed() {
            self.refresh(bot);
        }
    }

    /// 换了 Bot 清掉上一个的一切（在登录就取消）；同一个 Bot 是下拉刷新，页面照旧显示到结果回来。
    fn open(&self, bot: BotRef) {
        if self.viewed().as_ref() == Some(&bot) {
            let mut models = None;
            self.view.send_modify(|view| {
                if matches!(view.state, AgentModelsState::Failed { .. }) {
                    view.state = AgentModelsState::Loading;
                }
                if let Some(page) = view.provider.as_mut().filter(|page| page.row.connected) {
                    page.models = ModelListState::Loading;
                    models = Some(page.row.id.clone());
                }
            });
            self.job(bot.clone(), Op::Catalog, true);
            if let Some(provider) = models {
                self.enqueue(bot, Op::Models { provider });
            }
            return;
        }
        self.drop_login();
        lock(&self.local).catalog = None;
        self.view.send_replace(AgentModelsViewModel {
            bot: Some(bot.clone()),
            ..AgentModelsViewModel::default()
        });
        self.job(bot, Op::Catalog, true);
    }

    fn search(&self, text: String) {
        let catalog = self.catalog();
        self.view.send_modify(|view| {
            if let Some(catalog) = &catalog {
                view.more = view::rows(catalog, &text).1;
            }
            view.search = text;
        });
    }

    fn open_provider(&self, bot: &BotRef, id: &str) -> Result<(), CoreError> {
        let catalog = self.catalog().ok_or_else(invalid)?;
        let provider = view::find(&catalog, id).ok_or_else(invalid)?;
        let page = view::page(provider);
        let load = page.models == ModelListState::Loading;
        self.drop_login();
        self.view.send_modify(|view| view.provider = Some(page));
        if load {
            self.enqueue(
                bot.clone(),
                Op::Models {
                    provider: id.to_owned(),
                },
            );
        }
        Ok(())
    }

    /// 能回跳的走浏览器授权，否则走设备码；在登录就先取消那一次。
    fn start_login(&self, bot: BotRef) -> Result<(), CoreError> {
        let redirect = {
            let view = self.view.borrow();
            let page = view.provider.as_ref().ok_or_else(invalid)?;
            if !page.device_login && !page.browser_login {
                return Err(invalid());
            }
            page.browser_login
        };
        self.drop_login();
        let provider = self.busy()?;
        self.view.send_modify(|view| {
            if let Some(page) = view.provider.as_mut() {
                page.login_outcome = None;
            }
        });
        self.enqueue(bot, Op::LoginStart { provider, redirect });
        Ok(())
    }

    fn confirm(&self, bot: BotRef) -> Result<(), CoreError> {
        let mut select = None;
        self.view.send_modify(|view| {
            if let Some(page) = view.provider.as_mut()
                && let Some(confirm) = page.confirm.take()
            {
                page.busy = true;
                select = Some(SelectModel {
                    provider: page.row.id.clone(),
                    model: confirm.model,
                    confirmed: true,
                });
            }
        });
        self.enqueue(bot, Op::Select(select.ok_or_else(invalid)?));
        Ok(())
    }

    /// 取消正在进行的登录：页面上立刻清掉，取消请求在后台发，失败了也只记日志（会话到期插件自己会丢）。
    fn drop_login(&self) {
        let Some(login) = lock(&self.local).login.take() else {
            return;
        };
        self.view.send_modify(|view| {
            if let Some(page) = view.provider.as_mut() {
                page.login = None;
                page.busy = false;
            }
        });
        self.job(
            login.bot,
            Op::LoginCancel {
                session: login.session,
            },
            false,
        );
    }

    /// 点开那家的按钮先禁用，返回它的 ID；没点开时是壳发错了动作。
    fn busy(&self) -> Result<String, CoreError> {
        let mut provider = None;
        self.view.send_modify(|view| {
            if let Some(page) = view.provider.as_mut() {
                page.busy = true;
                provider = Some(page.row.id.clone());
            }
        });
        provider.ok_or_else(invalid)
    }

    fn refresh(&self, bot: BotRef) {
        self.job(bot, Op::Catalog, false);
    }

    fn catalog(&self) -> Option<ModelCatalog> {
        let viewed = self.viewed()?;
        lock(&self.local)
            .catalog
            .as_ref()
            .filter(|(bot, _)| *bot == viewed)
            .map(|(_, catalog)| catalog.clone())
    }

    fn api(&self) -> Result<api::Api, wonton_protocol::AgentModelsProblem> {
        // 已知隧道不通就立刻失败，不等超时。
        if let Some(reason) = self.home.tunnel_outage() {
            return Err(api::transport(reason));
        }
        let proxy = self.home.view().proxy.ok_or_else(api::not_connected)?;
        let base = self.home.core_url().ok_or_else(api::not_connected)?;
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

    fn enqueue(&self, bot: BotRef, op: Op) {
        self.job(bot, op, true);
    }

    fn job(&self, bot: BotRef, op: Op, report: bool) {
        // 只在运行时关停、工作任务已经没了时发不出去，那时也没有界面等结果。
        let _ = self.jobs.send(Job { bot, op, report });
    }
}

async fn work(link: Weak<AgentModelsLink>, mut queued: mpsc::UnboundedReceiver<Job>) {
    while let Some(job) = queued.recv().await {
        let Some(link) = link.upgrade() else { return };
        link.run(job).await;
    }
}

fn invalid() -> CoreError {
    CoreError::InvalidInput {
        reason: l10n!(CATALOG, language(), "models-invalid-action"),
    }
}

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 连上家里的服务器（启动、断线恢复）时，正看着的 Bot 重新取一次目录。
async fn follow_home(link: Weak<AgentModelsLink>) {
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
            strong.refresh(bot);
        }
        connected = now_connected;
        drop(strong);
        if home.changed().await.is_err() {
            return;
        }
    }
}
