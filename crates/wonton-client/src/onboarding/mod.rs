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
use wonton_protocol::{
    CoreError, HomeStatus, Me, ModelSettings, ModelSettingsProblem, ONBOARDING_PATH, Onboarding,
    OnboardingAction, OnboardingCommit, OnboardingStep, OnboardingTip, OnboardingTooOld,
    OnboardingViewModel, PluginFailure, PluginProblem, minimum_server_version,
};

use crate::chat::ChatLink;
use crate::home::HomeLink;
use crate::plugins::PluginsLink;
use crate::scope::reuse;
use draft::Draft;
use review::ReviewEntry;
use storage::OnboardingStorage;
use view::render;

/// 谁要的这趟重新取。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Trigger {
    /// 主人要的：从网关网页回来、出错后点重试。设备被拒绝过就先再拨一次，主人可能刚把它加回去。
    Owner,
    /// 重看引导后的那趟，也是主人要的；失败另外处理，见 [`OnboardingLink::refresh_after_local_reset`]。
    AfterReset,
    /// 跟着连接、插件列表、语言变化自己取的：不重拨被拒绝的设备（ADR-0032）。决定取的时候还连着，派出去的任务
    /// 跑起来时设备可能刚被吊销；这时重拨，就成了被移除的设备自己连回去。
    Automatic,
}

#[derive(Default)]
struct LocalState {
    /// 家里服务器上的引导状态：走没走完和默认 Bot 插件。
    onboarding: Option<Onboarding>,
    /// 防止引导响应倒退；断线时递增以丢弃断线前发出的旧响应，不代表授权版本。
    revision: u64,
    /// 正在显示的一步。中间的步骤只在这台设备上（ADR-0038）；还没取到时为空。
    step: Option<OnboardingStep>,
    /// 从设置重看已经走完的引导：每一步都能不提交就离开，完成时只存动过的项。
    revisit: bool,
    owner: Option<Me>,
    model_settings: Option<ModelSettings>,
    draft: Draft,
    review: Vec<ReviewEntry>,
    /// 每次进出最后一页加一，旧的预览结果不落到新列出的项上。
    review_generation: u64,
    commit: OnboardingCommit,
    problem: Option<PluginProblem>,
    bot_refreshing: bool,
    bot_problem: Option<PluginProblem>,
    model_problem: Option<ModelSettingsProblem>,
    loading: bool,
    /// 这一趟里因为服务器太旧整步显不出来的步骤：这些步直接跳过。
    unavailable: HashSet<OnboardingStep>,
    /// 服务器比引导接口还旧：整段引导都不显示，只留一条升级提示。
    too_old: Option<OnboardingTooOld>,
    /// 连着却没取到之后自己又取了几次，定下一次等多久（[`OnboardingLink::retry_while_connected`]）。
    transport_retries: u32,
    /// 已经排了一次自己再取：同一时刻只排一次，主人点的重试和自动的不叠成两串。
    retry_scheduled: bool,
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
            self.refresh_automatically();
        } else if changed {
            self.publish();
        }
    }

    pub(crate) fn dispatch(self: &Arc<Self>, action: OnboardingAction) -> Result<(), CoreError> {
        match action {
            OnboardingAction::RefreshOnboarding => {
                self.refresh();
                Ok(())
            }
            OnboardingAction::RestartOnboarding => self.restart(),
            OnboardingAction::ContinueOnboarding => self.continue_step(),
            OnboardingAction::SkipOnboardingStep => self.skip_step(),
            OnboardingAction::BackOnboardingStep => self.back_step(),
            OnboardingAction::ToggleOnboardingNeed { id } => self.toggle_need(&id),
            OnboardingAction::EditOnboardingProfile {
                display_name,
                email,
            } => self.edit_profile(display_name, email),
            OnboardingAction::UseCurrentModel => self.use_current_model(),
            OnboardingAction::RetryOnboardingItem { id } => self.retry_item(&id),
            OnboardingAction::DismissOnboardingTip { tip } => self.dismiss_tip(tip),
            OnboardingAction::CloseOnboarding => {
                self.close();
                Ok(())
            }
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
        self.refresh_impl(Trigger::Owner);
    }

    /// 和 [`Self::refresh`] 一样重新取，但不是主人要的，见 [`Trigger::Automatic`]。
    fn refresh_automatically(self: &Arc<Self>) {
        self.refresh_impl(Trigger::Automatic);
    }

    /// 本地重置（重看引导）后的那趟和普通刷新一样，完整取到的资料、模型设置和错误必须落地。
    /// 插件变化触发的静默刷新只更新 onboarding，可能先一步增加 revision；它不能让这趟已取到的详情失效。
    /// 重置后的失败仍由 `after_reset` 单独处理，因为那是之后唯一会取资料和模型设置的请求。
    pub(super) fn refresh_after_local_reset(self: &Arc<Self>) {
        self.refresh_impl(Trigger::AfterReset);
    }

    fn refresh_impl(self: &Arc<Self>, trigger: Trigger) {
        let after_reset = trigger == Trigger::AfterReset;
        let (generation, operation, revision) = {
            let mut state = lock(&self.state);
            state.loading = true;
            // 上一次的问题留到这一趟有结果再换：重试时壳在“重试”上转圈，提示不先消失再原样冒出来。
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
            if trigger != Trigger::Automatic {
                link.home.retry_if_rejected().await;
            }
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
            // 走完了的不取资料和模型设置：没有地方显示它，重看引导时再取。
            let details = if in_flow {
                Some(tokio::join!(api::me(&api), api::settings(&api)))
            } else {
                None
            };
            if !link.operation_is_current(generation, operation) {
                return;
            }
            link.received(onboarding, details, revision);
        });
    }

    fn received(
        self: &Arc<Self>,
        onboarding: Onboarding,
        details: Option<(
            Result<Me, PluginProblem>,
            Result<ModelSettings, ModelSettingsProblem>,
        )>,
        revision: u64,
    ) {
        {
            let mut state = lock(&self.state);
            state.problem = None;
            state.transport_retries = 0;
            // 服务端的 Onboarding 不带版本号，按请求发出时的 revision 判新旧：这趟请求发出后已有
            // 更新的响应落过地（静默刷新、点完成的保存），这份旧的丢弃，视图保持已落地的状态。
            let current = state.revision == revision;
            if current {
                if state.step.is_none() {
                    // 中间的步骤不存在家里服务器上：没走完的从第一个能显示的步开始，每一步显示现在的设置。
                    state.revisit = onboarding.step == OnboardingStep::Finished;
                    state.step = Some(match onboarding.step {
                        OnboardingStep::Finished => OnboardingStep::Finished,
                        OnboardingStep::Other => OnboardingStep::Other,
                        _ => view::first_visible(&state.unavailable)
                            .unwrap_or(OnboardingStep::Profile),
                    });
                }
                state.onboarding = Some(onboarding);
                state.bot_refreshing = self.home.view().status != HomeStatus::Connected;
                state.bot_problem = None;
                state.revision += 1;
            }
            // 静默刷新只更新 onboarding；完整刷新已取到的详情仍是这趟请求的结果。
            if let Some((owner, model_settings)) = details {
                match owner {
                    Ok(owner) => state.owner = Some(owner),
                    Err(problem) => {
                        if problem.reason == PluginFailure::ServerTooOld {
                            state.unavailable.insert(OnboardingStep::Profile);
                            if state
                                .step
                                .is_some_and(|step| state.unavailable.contains(&step))
                            {
                                state.step = view::first_visible(&state.unavailable);
                            }
                        } else {
                            state.problem = Some(problem);
                        }
                    }
                }
                match model_settings {
                    Ok(model_settings) => state.model_settings = Some(model_settings),
                    Err(problem) => {
                        state.model_settings = None;
                        state.model_problem = Some(problem);
                    }
                }
            }
            state.loading = false;
        }
        self.publish();
        // 资料取不到也会挂着“连不上”，和状态取不到一样要自己再取。
        self.retry_while_connected();
    }

    fn finish_error(
        self: &Arc<Self>,
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
                state.bot_problem = Some(problem);
                state.problem = None;
                state.step = None;
            } else {
                state.bot_problem = Some(problem.clone());
                state.problem = Some(problem);
            }
            state.bot_refreshing = false;
        } else {
            // 这份错误比已落地的结果旧，丢掉；上一次的问题也跟着那份结果过时了。
            state.problem = None;
        }
        drop(state);
        self.publish();
        self.retry_while_connected();
    }

    fn dismiss_tip(&self, tip: OnboardingTip) -> Result<(), CoreError> {
        self.storage.dismiss_tip(tip)?;
        // 写成功说明本机记录已经完整，启动时“读不了记录”的问题不再成立。
        lock(&self.state).problem = None;
        self.publish();
        Ok(())
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
        if !records.prompts_used
            && !state.loading
            && !state.bot_refreshing
            && state.bot_problem.is_none()
            && view::prompt_done(state.onboarding.as_ref(), &chat, &sent_rooms)
        {
            match self.storage.mark_prompts_used() {
                Ok(()) => records.prompts_used = true,
                Err(error) => state.problem = Some(storage_problem(error)),
            }
        }
        let (view, default_bot) = render(&state, &records);
        #[cfg(feature = "test-hooks")]
        if let Some(hook) = lock(&self.test_publish_hook).as_ref() {
            hook(&view, &sent_rooms);
        }
        // 快照必须在持锁时发布，不能让较早渲染的 loading 覆盖完成状态。
        self.view.send_replace(view);
        drop(state);
        self.chat.set_default_bot(default_bot);
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
