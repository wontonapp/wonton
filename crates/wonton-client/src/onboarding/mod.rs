//! 新用户引导：每一步记下的选择、最后一页的提交和设备上的一次性提示（ADR-0034、ADR-0038）。

mod api;
mod draft;
mod follow;
mod model;
mod refresh;
mod review;
mod steps;
mod storage;
mod view;

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::runtime::Handle;
use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_protocol::{
    Action, CoreError, HomeStatus, Me, ModelProviderFailure, ModelProviderProblem, ModelProviders,
    ONBOARDING_PATH, Onboarding, OnboardingCommit, OnboardingStep, OnboardingTip, OnboardingTooOld,
    OnboardingUpdate, OnboardingViewModel, PluginFailure, PluginProblem, minimum_server_version,
};

use crate::chat::ChatLink;
use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};
use crate::plugins::PluginsLink;
use crate::scope::reuse;
use draft::Draft;
use review::ReviewEntry;
use storage::OnboardingStorage;
use view::render;

#[derive(Default)]
struct LocalState {
    /// 家里服务器上的引导状态：只记走没走完和默认 Agent。
    onboarding: Option<Onboarding>,
    /// 每落一份响应加一；刷新记下发出时的值，回来对不上就丢弃（Onboarding 没有服务端版本号）。
    revision: u64,
    /// 正在显示的一步。中间的步骤只在这台设备上（ADR-0038）；还没取到时为空。
    step: Option<OnboardingStep>,
    owner: Option<Me>,
    providers: Option<ModelProviders>,
    draft: Draft,
    testing: bool,
    review: Vec<ReviewEntry>,
    /// 每次进出最后一页加一，旧的预览结果不落到新列出的项上。
    review_generation: u64,
    commit: OnboardingCommit,
    problem: Option<PluginProblem>,
    model_problem: Option<ModelProviderProblem>,
    loading: bool,
    /// 这一趟里因为服务器太旧整步显不出来的步骤：这些步直接跳过。
    unavailable: HashSet<OnboardingStep>,
    /// 服务器比引导接口还旧：整段引导都不显示，只留一条升级提示。
    too_old: Option<OnboardingTooOld>,
}

/// `test-hooks` 下发布前交给测试的回调：即将发出去的视图和已记下发过消息的房间。
#[cfg(feature = "test-hooks")]
pub(crate) type TestPublishHook = Arc<dyn Fn(&OnboardingViewModel, &HashSet<String>) + Send + Sync>;

pub(crate) struct OnboardingLink {
    runtime: Handle,
    home: Arc<HomeLink>,
    chat: Arc<ChatLink>,
    plugins: Arc<PluginsLink>,
    view: watch::Sender<OnboardingViewModel>,
    state: Mutex<LocalState>,
    storage: OnboardingStorage,
    /// 上次告诉核心的语言，变了才重新取：家里的服务器按 `Accept-Language` 挑引导里的名字、问题和服务商。
    languages: Mutex<Vec<String>>,
    /// 换服务器时加一，之前发起的请求的结果都作废。
    generation: AtomicU64,
    /// 每次取状态或者在设置里改引导状态时加一，只认最后一次的结果。
    operation: AtomicU64,
    sent_rooms: Mutex<HashSet<String>>,
    /// 每次发布前先把即将发出去的视图和记下发过消息的房间交给它（[`ClientCore::test_set_onboarding_publish_hook`]）。
    /// [`ClientCore::test_set_onboarding_publish_hook`]: crate::ClientCore::test_set_onboarding_publish_hook
    #[cfg(feature = "test-hooks")]
    pub(crate) test_publish_hook: Mutex<Option<TestPublishHook>>,
}

impl OnboardingLink {
    pub(crate) fn new(
        runtime: Handle,
        home: Arc<HomeLink>,
        chat: Arc<ChatLink>,
        plugins: Arc<PluginsLink>,
        data_dir: &Path,
        view: Option<watch::Sender<OnboardingViewModel>>,
    ) -> Arc<Self> {
        let (storage, problem) = OnboardingStorage::open(data_dir);
        let sent_rooms = storage.snapshot().prompt_rooms.into_iter().collect();
        let link = Arc::new(Self {
            runtime: runtime.clone(),
            home: home.clone(),
            chat: chat.clone(),
            plugins,
            view: reuse(view, OnboardingViewModel::default()),
            state: Mutex::new(LocalState {
                problem: problem.map(storage_problem),
                ..LocalState::default()
            }),
            storage,
            languages: Mutex::new(Vec::new()),
            generation: AtomicU64::new(0),
            operation: AtomicU64::new(0),
            sent_rooms: Mutex::new(sent_rooms),
            #[cfg(feature = "test-hooks")]
            test_publish_hook: Mutex::new(None),
        });
        link.publish();
        follow::spawn(&link, &runtime, &home, &chat);
        runtime.spawn(refresh::follow_plugins(
            Arc::downgrade(&link),
            link.plugins.subscribe(),
        ));
        link
    }

    pub(crate) fn view(&self) -> OnboardingViewModel {
        self.view.borrow().clone()
    }

    /// 换服务器时新作用域接着用这个发送端，壳的订阅不断（ADR-0042）。
    pub(crate) fn sender(&self) -> watch::Sender<OnboardingViewModel> {
        self.view.clone()
    }

    pub(crate) fn set_languages(self: &Arc<Self>, languages: Vec<String>) {
        let changed = {
            let mut current = lock(&self.languages);
            if *current == languages {
                false
            } else {
                *current = languages;
                true
            }
        };
        if changed && self.home.view().status == HomeStatus::Connected {
            self.refresh();
        } else if changed {
            self.publish();
        }
    }

    pub(crate) fn dispatch(self: &Arc<Self>, action: Action) -> Result<(), CoreError> {
        match action {
            Action::RefreshOnboarding => {
                self.refresh();
                Ok(())
            }
            Action::RestartOnboarding => self.restart(),
            Action::ChooseOnboardingAgent { id } => self.choose_agent(&id),
            Action::ContinueOnboarding => self.continue_step(),
            Action::SkipOnboardingStep => self.skip_step(),
            Action::BackOnboardingStep => self.back_step(),
            Action::ToggleOnboardingNeed { id } => self.toggle_need(&id),
            Action::EditOnboardingProfile {
                display_name,
                email,
            } => self.edit_profile(display_name, email),
            Action::TestModelProvider {
                input,
                then_continue,
            } => self.test_provider(input, then_continue),
            Action::UseCurrentModel => self.use_current_model(),
            Action::ChooseModelLogin { id } => self.choose_login(&id),
            Action::RetryOnboardingItem { id } => self.retry_item(&id),
            Action::DismissModelProviderProblem => {
                lock(&self.state).model_problem = None;
                self.publish();
                Ok(())
            }
            Action::SetPrimaryAgent { id } => self.set_primary(&id),
            Action::DismissReplacedAgentOffer => self.dismiss_replaced(),
            Action::DismissOnboardingTip { tip } => self.dismiss_tip(tip),
            _ => Err(invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-invalid-action"
            ))),
        }
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn begin_operation(&self) -> (u64, u64) {
        (
            self.generation(),
            self.operation.fetch_add(1, Ordering::SeqCst) + 1,
        )
    }

    fn operation_is_current(&self, generation: u64, operation: u64) -> bool {
        self.generation() == generation && self.operation.load(Ordering::SeqCst) == operation
    }

    /// 重新取家里服务器上的状态。记下的选择和正在显示的一步不变：从网关网页回来、出错后重试都走这里。
    fn refresh(self: &Arc<Self>) {
        self.refresh_impl(false);
    }

    /// 本地重置（重看引导）后的那一趟。重置不写服务器，它是之后唯一取资料和服务商的请求；插件变化触发的
    /// 静默刷新（只取 onboarding）可能后发先至，把这趟按 revision 判旧——onboarding 照旧让给更新的那份，
    /// 资料、服务商和错误却没有更新的可让，必须落地，否则重看的每一步都没有现值可显示。
    pub(super) fn refresh_after_local_reset(self: &Arc<Self>) {
        self.refresh_impl(true);
    }

    fn refresh_impl(self: &Arc<Self>, after_reset: bool) {
        let (generation, operation, revision) = {
            let mut state = lock(&self.state);
            state.loading = true;
            state.problem = None;
            state.model_problem = None;
            // 每次重新取都从头判：服务器升级了，踢掉的步会回来。
            state.unavailable.clear();
            state.too_old = None;
            let (generation, operation) = self.begin_operation();
            (generation, operation, state.revision)
        };
        self.publish();
        let link = self.clone();
        self.runtime.spawn(async move {
            link.home.retry_if_rejected().await;
            let result = match link.plugins.api() {
                Ok(api) => api::get(&api).await.map(|onboarding| (api, onboarding)),
                Err(problem) => Err(problem),
            };
            if !link.operation_is_current(generation, operation) {
                return;
            }
            let (api, onboarding) = match result {
                Ok(fetched) => fetched,
                Err(problem) => {
                    return link.finish_error(
                        generation,
                        operation,
                        revision,
                        after_reset,
                        problem,
                    );
                }
            };
            let in_flow = {
                let state = lock(&link.state);
                let step = state.step.unwrap_or(onboarding.step);
                !matches!(step, OnboardingStep::Finished | OnboardingStep::Other)
            };
            // 走完了的不取资料和服务商：没有地方显示它们，重看引导时再取。
            let details = if in_flow {
                Some(tokio::join!(api::me(&api), api::providers(&api)))
            } else {
                None
            };
            if !link.operation_is_current(generation, operation) {
                return;
            }
            link.received(onboarding, details, revision, after_reset);
        });
    }

    fn received(
        &self,
        onboarding: Onboarding,
        details: Option<(
            Result<Me, PluginProblem>,
            Result<ModelProviders, ModelProviderProblem>,
        )>,
        revision: u64,
        after_reset: bool,
    ) {
        {
            let mut state = lock(&self.state);
            // 服务端的 Onboarding 不带版本号，按请求发出时的 revision 判新旧：这趟请求发出后已有
            // 更新的响应落过地（静默刷新、点完成的保存），这份旧的丢弃，视图保持已落地的状态。
            let current = state.revision == revision;
            if current {
                if state.step.is_none() {
                    // 中间的步骤不存在家里服务器上：没走完的从第一个能显示的步开始，每一步显示现在的设置。
                    state.step = Some(match onboarding.step {
                        OnboardingStep::Finished => OnboardingStep::Finished,
                        OnboardingStep::Other => OnboardingStep::Other,
                        _ => view::first_visible(&state.unavailable)
                            .unwrap_or(OnboardingStep::Profile),
                    });
                }
                state.onboarding = Some(onboarding);
                state.revision += 1;
            }
            if (current || after_reset)
                && let Some((owner, providers)) = details
            {
                match owner {
                    Ok(owner) => state.owner = Some(owner),
                    Err(problem) => {
                        if problem.reason == PluginFailure::ServerTooOld {
                            state.unavailable.insert(OnboardingStep::Profile);
                            state.step = view::first_visible(&state.unavailable);
                        } else {
                            state.problem = Some(problem);
                        }
                    }
                }
                match providers {
                    Ok(providers) => state.providers = Some(providers),
                    Err(problem) => state.model_problem = Some(problem),
                }
            }
            state.loading = false;
        }
        self.publish();
    }

    fn finish_error(
        &self,
        generation: u64,
        operation: u64,
        revision: u64,
        after_reset: bool,
        problem: PluginProblem,
    ) {
        if !self.operation_is_current(generation, operation) {
            return;
        }
        let mut state = lock(&self.state);
        state.loading = false;
        // 请求发出后已有更新的响应落过地：这份旧错误丢弃，视图保持已落地的状态。
        // 重置后的那趟除外：它的失败没有别的请求会报（见 refresh_after_local_reset）。
        if after_reset || state.revision == revision {
            if problem.reason == PluginFailure::ServerTooOld {
                state.too_old = Some(too_old(&problem));
                state.problem = None;
                state.step = None;
            } else {
                state.problem = Some(problem);
            }
        }
        drop(state);
        self.publish();
    }

    fn set_primary(self: &Arc<Self>, id: &str) -> Result<(), CoreError> {
        let state = lock(&self.state);
        let onboarding = state.onboarding.as_ref().ok_or_else(|| {
            invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-steps-state-missing"
            ))
        })?;
        let agent = onboarding
            .agents
            .iter()
            .find(|agent| agent.id == id)
            .ok_or_else(|| {
                invalid(&l10n!(
                    CATALOG,
                    language(),
                    "onboarding-steps-agent-not-listed"
                ))
            })?;
        if !view::installable(agent) {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-steps-agent-not-installable"
            )));
        }
        drop(state);
        self.spawn_update(OnboardingUpdate {
            primary_agent: Some(id.to_owned()),
            ..OnboardingUpdate::default()
        });
        Ok(())
    }

    fn dismiss_replaced(self: &Arc<Self>) -> Result<(), CoreError> {
        if lock(&self.state).onboarding.is_none() {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "onboarding-steps-state-missing"
            )));
        }
        self.spawn_update(OnboardingUpdate {
            replaced_agent_asked: Some(true),
            ..OnboardingUpdate::default()
        });
        Ok(())
    }

    fn dismiss_tip(&self, tip: OnboardingTip) -> Result<(), CoreError> {
        self.storage.dismiss_tip(tip)?;
        // 写成功说明本机记录已经完整，启动时“读不了记录”的问题不再成立。
        lock(&self.state).problem = None;
        self.publish();
        Ok(())
    }

    /// 设置里的改动（默认 Agent、要不要卸掉换下来的）直接保存，不走引导最后一起提交。
    fn spawn_update(self: &Arc<Self>, update: OnboardingUpdate) {
        let (generation, operation) = {
            let mut state = lock(&self.state);
            state.loading = true;
            state.problem = None;
            self.begin_operation()
        };
        self.publish();
        let link = self.clone();
        self.runtime.spawn(async move {
            let result = match link.plugins.api() {
                Ok(api) => api::update(&api, &update).await,
                Err(problem) => Err(problem),
            };
            if !link.operation_is_current(generation, operation) {
                return;
            }
            let mut state = lock(&link.state);
            state.loading = false;
            match result {
                Ok(onboarding) => {
                    state.onboarding = Some(onboarding);
                    state.revision += 1;
                }
                Err(problem) if problem.reason == PluginFailure::ServerTooOld => {
                    state.too_old = Some(too_old(&problem));
                    state.step = None;
                }
                Err(problem) => state.problem = Some(problem),
            }
            drop(state);
            link.publish();
        });
    }

    fn server_changed(&self, server_id: Option<String>) {
        {
            let mut state = lock(&self.state);
            self.generation.fetch_add(1, Ordering::SeqCst);
            *state = LocalState::default();
        }
        lock(&self.sent_rooms).clear();
        if let Some(server_id) = server_id
            && let Err(error) = self.storage.sync_server(&server_id)
        {
            lock(&self.state).problem = Some(storage_problem(error));
        }
        self.publish();
    }

    fn publish(&self) {
        let mut state = lock(&self.state);
        let mut records = self.storage.snapshot();
        let chat = self.chat.view();
        let sent_rooms = lock(&self.sent_rooms).clone();
        // 发过消息先落盘再渲染：收起提问的视图发出去时 `prompts_used` 一定已经在盘上。落盘和渲染用
        // 同一份快照，房间行或私聊标记短暂缺失时不落盘也不收起，回来后的发布再收。
        if !records.prompts_used && view::prompt_done(state.onboarding.as_ref(), &chat, &sent_rooms)
        {
            match self.storage.mark_prompts_used() {
                Ok(()) => records.prompts_used = true,
                Err(error) => state.problem = Some(storage_problem(error)),
            }
        }
        let (view, primary_bot) = render(&state, &records);
        #[cfg(feature = "test-hooks")]
        if let Some(hook) = lock(&self.test_publish_hook).as_ref() {
            hook(&view, &sent_rooms);
        }
        // 快照必须在持锁时发布，不能让较早渲染的 loading 覆盖完成状态。
        self.view.send_replace(view);
        drop(state);
        self.chat.set_primary_bot(primary_bot);
    }
}

fn provider_problem(problem: PluginProblem) -> ModelProviderProblem {
    ModelProviderProblem {
        reason: match problem.reason {
            PluginFailure::TransportUnavailable => ModelProviderFailure::TransportUnavailable,
            PluginFailure::Unauthorized => ModelProviderFailure::Unauthorized,
            PluginFailure::Protocol => ModelProviderFailure::Protocol,
            _ => ModelProviderFailure::Upstream,
        },
        message: problem.message,
    }
}

fn storage_problem(error: CoreError) -> PluginProblem {
    PluginProblem {
        reason: PluginFailure::Upstream,
        min_version: None,
        message: error.to_string(),
        settings: None,
    }
}

/// 服务器比引导接口还旧：提示里带上要的版本。
fn too_old(problem: &PluginProblem) -> OnboardingTooOld {
    OnboardingTooOld {
        min_version: problem
            .min_version
            .clone()
            .or_else(|| minimum_server_version(ONBOARDING_PATH).map(str::to_owned)),
    }
}

fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidInput {
        reason: reason.to_owned(),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
